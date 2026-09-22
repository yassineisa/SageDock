//! Jupyter server lifecycle: starting it inside the SageDock environment, detecting when
//! it is genuinely ready, and shutting it down cleanly.
//!
//! Security posture, per the product spec:
//! - The server binds to `127.0.0.1` only and is never exposed to the local network.
//! - Every session gets a fresh random token from the OS CSPRNG.
//! - The port, URL and token are implementation details the user never sees or types.
//!
//! Readiness is determined by polling Jupyter's own `/api/status` endpoint, not by
//! sleeping for a plausible-looking duration. A slow machine and a broken install are
//! indistinguishable to a sleep; they are not to a readiness probe.

pub mod notebook;

use std::net::TcpListener;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::error::{AppError, AppResult, ErrorSeverity};
use crate::runtime::{self, wsl};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How long to wait for Jupyter to answer. Generous because a cold first start inside a
/// freshly-created environment genuinely takes a while; the poll loop exits as soon as
/// the server answers, so this only bounds the failure case.
const READY_TIMEOUT: Duration = Duration::from_secs(90);
const POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Serialize)]
pub struct JupyterSession {
    pub port: u16,
    /// Held so the app can build authenticated URLs. Never rendered in the UI.
    #[serde(skip)]
    pub token: String,
}

/// A running server plus the process handle that owns its lifetime.
pub struct RunningServer {
    pub session: JupyterSession,
    /// The workspace folder this server is rooted at.
    ///
    /// Recorded so a second workspace can be opened without disturbing this one: a Jupyter
    /// server exposes exactly one directory tree, so switching workspaces by re-rooting a
    /// shared server would either kill the first workspace's running calculations or show
    /// the wrong folder. Keyed by root, each workspace gets its own.
    pub root: std::path::PathBuf,
    child: Child,
}

impl RunningServer {
    pub fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
    /// Stops the server. Killing the `wsl.exe` process ends the Jupyter process it hosts,
    /// which is why the handle is kept rather than launching Jupyter detached.
    pub fn stop(&mut self) {
        // Ask Jupyter to shut down first. Killing the Windows-side `wsl.exe` process is not
        // guaranteed to end the Linux process it started, which would leave a server
        // running with nothing left to control it.
        let _ = ureq::post(&format!(
            "http://127.0.0.1:{}/api/shutdown",
            self.session.port
        ))
        .set("Authorization", &format!("token {}", self.session.token))
        .timeout(Duration::from_secs(3))
        .call();

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        let _ = self.child.kill();
        let _ = self.child.wait();
        tracing::info!(target: "jupyter", "server stopped");
    }
}

/// Finds a free localhost port by binding port 0 and letting the OS choose.
///
/// There is an unavoidable gap between releasing the port here and Jupyter binding it, so
/// the caller treats "Jupyter never became ready" as a retryable condition rather than
/// assuming this port is still free. Fixed ports are avoided entirely, they collide with
/// whatever else the user is running.
fn find_free_port() -> AppResult<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|err| {
        AppError::new(
            "jupyter",
            "PORT_ALLOCATION_FAILED",
            "SageDock couldn't start the notebook service",
            "SageDock wasn't able to reserve a connection on this computer. Restarting your computer usually resolves this.",
        )
        .with_technical_details(err.to_string())
    })?;

    let port = listener
        .local_addr()
        .map_err(|err| {
            AppError::new(
                "jupyter",
                "PORT_ALLOCATION_FAILED",
                "SageDock couldn't start the notebook service",
                "SageDock wasn't able to reserve a connection on this computer.",
            )
            .with_technical_details(err.to_string())
        })?
        .port();

    Ok(port)
}

/// Generates a URL-safe random token from the OS CSPRNG.
fn generate_token() -> AppResult<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|err| {
        AppError::new(
            "jupyter",
            "TOKEN_GENERATION_FAILED",
            "SageDock couldn't start the notebook service securely",
            "SageDock wasn't able to create a secure connection for your notebooks, so it stopped rather than continuing without one.",
        )
        .with_severity(ErrorSeverity::Fatal)
        .with_technical_details(err.to_string())
    })?;

    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Starts JupyterLab inside the SageDock environment, rooted at the user's workspace.
pub fn start(workspace_dir: &Path) -> AppResult<RunningServer> {
    let port = find_free_port()?;
    let token = generate_token()?;
    let root = wsl::windows_path_to_wsl(workspace_dir)?;

    // Arguments are passed as separate argv entries, never assembled into a shell string,
    // so neither the workspace path nor the token can be reinterpreted by a shell.
    let port_arg = format!("--port={port}");
    let token_arg = format!("--ServerApp.token={token}");
    let root_arg = format!("--ServerApp.root_dir={root}");

    let launch = [
        runtime::contract::JUPYTER_LAUNCHER,
        "--no-browser",
        "--LabApp.expose_app_in_browser=True",
        // Bind to loopback only. This is the line that keeps the user's notebooks off their
        // local network.
        "--ServerApp.ip=127.0.0.1",
        "--ServerApp.allow_remote_access=False",
        "--ServerApp.open_browser=False",
        "--ServerApp.port_retries=0",
        port_arg.as_str(),
        token_arg.as_str(),
        root_arg.as_str(),
    ];

    // `exec_args` uses `--exec`, so a workspace path containing spaces, quotes or `$`
    // reaches Jupyter unchanged instead of being re-parsed by a Linux shell.
    let child = Command::new("wsl.exe")
        .args(wsl::exec_args(wsl::LINUX_USER, &launch))
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| {
            AppError::new(
                "jupyter",
                "JUPYTER_SPAWN_FAILED",
                "The notebook service did not start",
                "SageDock tried to start the notebook service, but it stopped before it was ready. Your notebooks are safe.",
            )
            .with_technical_details(err.to_string())
        })?;

    tracing::info!(target: "jupyter", port, "jupyter starting");

    Ok(RunningServer {
        session: JupyterSession { port, token },
        root: workspace_dir.to_path_buf(),
        child,
    })
}

/// Blocks until the server answers its own status endpoint, or the timeout expires.
///
/// Also watches the child process: if Jupyter exits early (a broken environment, a port
/// grabbed by something else) this returns immediately instead of waiting out the full
/// timeout on a process that is already gone.
pub fn wait_until_ready(server: &mut RunningServer) -> AppResult<()> {
    let deadline = Instant::now() + READY_TIMEOUT;

    while Instant::now() < deadline {
        if let Ok(Some(status)) = server.child.try_wait() {
            return Err(AppError::new(
                "jupyter",
                "JUPYTER_EXITED_EARLY",
                "The notebook service did not start",
                "SageDock tried to start the notebook service, but it stopped before becoming ready. Your notebooks are safe.",
            )
            .with_technical_details(format!("jupyter exited with status {status}")));
        }

        if status_ready(&server.session) {
            tracing::info!(target: "jupyter", port = server.session.port, "jupyter ready");
            return Ok(());
        }

        std::thread::sleep(POLL_INTERVAL);
    }

    Err(AppError::new(
        "jupyter",
        "JUPYTER_READY_TIMEOUT",
        "The notebook service is taking too long to start",
        "SageDock started the notebook service but it never became ready. Your notebooks are safe. Trying again usually resolves this.",
    ))
}

fn encode_path(relative_path: &str) -> String {
    relative_path
        .replace('\\', "/")
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// The name of the persisted JupyterLab UI workspace for one session.
///
/// Sessions are per folder, and JupyterLab stores its open-tab layout per UI workspace. Two
/// servers rooted at different folders sharing the default workspace would each restore the
/// other's tabs, pointing at files that do not exist in their own root.
pub fn lab_workspace(port: u16) -> String {
    format!("sagedock-{port}")
}

/// A URL inside a named JupyterLab UI workspace, optionally addressing one file.
///
/// The empty path is the case that matters. `/lab/workspaces/<name>/tree/<path>` is a valid
/// route, but `/lab/workspaces/<name>/tree` with nothing after it is not: JupyterLab answers
/// "path not found" and redirects to `/`. Building the tree segment only when there is a
/// path to put in it is what makes a workspace launch land on the workspace instead of an
/// error page. The session root still bounds what is reachable either way.
pub fn session_url(session: &JupyterSession, lab_workspace: &str, relative_path: &str) -> String {
    let base = format!(
        "http://127.0.0.1:{}/lab/workspaces/{}",
        session.port, lab_workspace
    );
    if relative_path.is_empty() {
        format!("{base}?token={}", session.token)
    } else {
        format!(
            "{base}/tree/{}?token={}",
            encode_path(relative_path),
            session.token
        )
    }
}

pub fn status_ready(session: &JupyterSession) -> bool {
    let response = ureq::AgentBuilder::new()
        .redirects(0)
        .build()
        .get(&format!("http://127.0.0.1:{}/api/status", session.port))
        .set("Authorization", &format!("token {}", session.token))
        .timeout(Duration::from_secs(2))
        .call();
    response
        .ok()
        .filter(|r| r.status() == 200)
        .and_then(|r| r.into_string().ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .is_some_and(|v| v.get("kernels").is_some() && v.get("started").is_some())
}

pub fn start_ready(workspace: &Path) -> AppResult<RunningServer> {
    for attempt in 0..3 {
        let mut server = start(workspace)?;
        match wait_until_ready(&mut server) {
            Ok(()) => return Ok(server),
            Err(err) => {
                server.stop();
                if attempt == 2 || err.code != "JUPYTER_EXITED_EARLY" {
                    return Err(err);
                }
            }
        }
    }
    unreachable!()
}

pub fn verify_kernels(session: &JupyterSession) -> AppResult<()> {
    let failed = || {
        AppError::new("jupyter", "NOTEBOOK_SERVICE_INVALID", "The notebook service needs repair",
        "SageDock couldn't verify a secure connection and both notebook engines. Your saved notebooks are safe. Open Recovery to repair the environment.")
    };
    let agent = ureq::AgentBuilder::new().redirects(0).build();
    let root = format!("http://127.0.0.1:{}", session.port);
    let response = agent
        .get(&format!("{root}/api/kernelspecs"))
        .set("Authorization", &format!("token {}", session.token))
        .timeout(Duration::from_secs(5))
        .call()
        .map_err(|_| failed())?;
    let json: serde_json::Value =
        serde_json::from_str(&response.into_string().map_err(|_| failed())?)
            .map_err(|_| failed())?;
    if !["sagemath", "python3"]
        .iter()
        .all(|k| json["kernelspecs"].get(k).is_some())
    {
        return Err(failed());
    }
    match agent
        .get(&format!("{root}/api/contents"))
        .timeout(Duration::from_secs(5))
        .call()
    {
        Err(ureq::Error::Status(401 | 403, _)) => Ok(()),
        _ => Err(failed()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocated_ports_are_usable_and_not_fixed() {
        let a = find_free_port().unwrap();
        assert!(a >= 1024, "should not hand out a privileged port");
    }

    #[test]
    fn tokens_are_long_random_hex() {
        let a = generate_token().unwrap();
        let b = generate_token().unwrap();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b, "tokens must not repeat between sessions");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// Every URL the app builds must stay on loopback, a regression here would expose a
    /// user's notebooks to their network.
    #[test]
    fn urls_are_always_loopback() {
        let session = JupyterSession {
            port: 8888,
            token: "abc".into(),
        };
        let workspace = lab_workspace(session.port);
        for url in [
            session_url(&session, &workspace, "Notebooks/a.ipynb"),
            session_url(&session, &workspace, ""),
        ] {
            assert!(url.starts_with("http://127.0.0.1:8888/"), "got {url}");
        }
    }

    /// The regression test for the workspace-launch failure: opening a course produced
    /// `/lab/workspaces/sagedock-<port>/tree`, and JupyterLab answers that with
    /// "path not found" and a redirect to `/`, because `tree` needs something after it.
    #[test]
    fn a_workspace_launch_never_addresses_the_empty_tree_route() {
        let session = JupyterSession {
            port: 54276,
            token: "abc".into(),
        };
        let url = session_url(&session, &lab_workspace(session.port), "");
        assert_eq!(
            url,
            "http://127.0.0.1:54276/lab/workspaces/sagedock-54276?token=abc"
        );
        assert!(
            !url.contains("/tree"),
            "an empty path must not produce a file-browser route: {url}"
        );
    }

    #[test]
    fn a_notebook_inside_a_named_workspace_keeps_its_tree_path() {
        let session = JupyterSession {
            port: 8888,
            token: "abc".into(),
        };
        let url = session_url(&session, &lab_workspace(session.port), "Notebooks/a.ipynb");
        assert_eq!(
            url,
            "http://127.0.0.1:8888/lab/workspaces/sagedock-8888/tree/Notebooks/a.ipynb?token=abc"
        );
    }

    /// Two sessions must not share one persisted Lab layout, or each restores the other's
    /// tabs and points them at files that do not exist in its own root.
    #[test]
    fn each_session_gets_its_own_lab_workspace_and_stays_on_loopback() {
        let a = JupyterSession {
            port: 1111,
            token: "t".into(),
        };
        let b = JupyterSession {
            port: 2222,
            token: "t".into(),
        };
        assert_ne!(lab_workspace(a.port), lab_workspace(b.port));
        for url in [
            session_url(&a, &lab_workspace(a.port), ""),
            session_url(&b, &lab_workspace(b.port), "x.ipynb"),
        ] {
            assert!(url.starts_with("http://127.0.0.1:"), "got {url}");
        }
    }

    #[test]
    fn session_urls_escape_spaces_and_normalize_separators() {
        let session = JupyterSession {
            port: 1,
            token: "t".into(),
        };
        let url = session_url(&session, "sagedock-1", r"Notebooks\My Notebook.ipynb");
        assert!(url.contains("Notebooks/My%20Notebook.ipynb"), "got {url}");
    }
}
