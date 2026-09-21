//! Integration tests for runtime installation, kernel execution, and Windows file access.
//!
//! Ignored by default because it needs a built runtime image, several gigabytes of disk,
//! and minutes to run. Unit tests cover parsers and filesystem safety separately; these
//! checks exercise the actual WSL runtime and authenticated Jupyter service.
//!
//! Run it with:
//!   powershell -File scripts/test-fresh-install.ps1
//!
//! Pass `-Image <path>` to the script to select a different staged runtime image.

use std::path::PathBuf;
use std::time::Instant;

use crate::jupyter;
use crate::jupyter::notebook::{create_notebook, NotebookKind};
use crate::runtime::provision::{run_setup, SetupOutcome, SetupPaths};

fn default_image() -> PathBuf {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    PathBuf::from(home)
        .join("SageDock-Runtime")
        .join("sagedock-runtime-sage10.9-x64.tar.xz")
}

fn scratch_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sagedock-e2e-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Uses the runner's isolated app-data and workspace paths, or temporary directories
/// when invoked directly. Never point these overrides at a student's existing files.
fn target_dir(env_var: &str, label: &str) -> PathBuf {
    match std::env::var(env_var) {
        Ok(path) if !path.is_empty() => {
            let dir = PathBuf::from(path);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }
        _ => scratch_dir(label),
    }
}

/// Refuse integration operations against a student's registered environment.
fn require_qa_environment() {
    let name = crate::runtime::wsl::distro_name();
    assert!(
        name.starts_with("SageDockQA-"),
        "Use scripts/test-fresh-install.ps1 for isolation"
    );
    assert!(
        crate::runtime::wsl::distro_exists(name),
        "run the install test first"
    );
}

#[test]
#[ignore]
fn installs_a_package_and_opens_a_sage_notebook() {
    assert!(crate::runtime::wsl::distro_name().starts_with("SageDockQA-"),
        "Set SAGEDOCK_QA_DISTRO to a new SageDockQA-* name. Never run installation tests against personal environments.");
    let image = std::env::var("SAGEDOCK_TEST_IMAGE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_image());

    assert!(
        image.is_file(),
        "no runtime image at {} — build one with scripts/build-runtime.ps1",
        image.display()
    );

    let paths = SetupPaths {
        app_data_dir: target_dir("SAGEDOCK_TEST_APPDATA", "appdata"),
        workspace_dir: target_dir("SAGEDOCK_TEST_WORKSPACE", "workspace"),
    };
    assert!(
        !crate::runtime::wsl::distro_exists(crate::runtime::wsl::distro_name()),
        "Fresh-install test requires an unused QA distro name"
    );
    println!("installing into {}", paths.app_data_dir.display());

    // --- setup ---------------------------------------------------------------------
    let started = Instant::now();
    let outcome = run_setup(&paths, Some(&image), "e2e", &mut |progress| {
        println!(
            "[{:>4}s] {:<38} {}",
            started.elapsed().as_secs(),
            progress.title,
            progress.detail.unwrap_or_default()
        );
    })
    .expect("setup should succeed");

    assert_eq!(
        outcome,
        SetupOutcome::Ready,
        "setup did not reach a ready state"
    );
    println!("setup finished in {}s", started.elapsed().as_secs());
    assert_eq!(
        std::fs::read_dir(&paths.workspace_dir).unwrap().count(),
        0,
        "setup must leave a plain empty folder, not a directory template"
    );

    // --- create a Sage notebook -----------------------------------------------------
    let relative_path = create_notebook(&paths.workspace_dir, NotebookKind::Sage)
        .expect("should create a notebook");
    assert!(paths.workspace_dir.join(&relative_path).is_file());
    println!("created {relative_path}");

    // --- start the notebook service and open it --------------------------------------
    let mut server = jupyter::start(&paths.workspace_dir).expect("jupyter should start");
    let ready = jupyter::wait_until_ready(&mut server);
    if let Err(err) = &ready {
        server.stop();
        panic!(
            "jupyter never became ready: {} / {:?}",
            err.message, err.technical_details
        );
    }
    println!("jupyter ready on port {}", server.session.port);
    jupyter::verify_kernels(&server.session).expect("Sage and Python kernels should execute");

    // A second course must get its own root while the first remains usable.
    let other = paths.workspace_dir.parent().unwrap().join("Physics $ test");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("physics.txt"), "second workspace").unwrap();
    let mut other_server = jupyter::start_ready(&other).expect("second workspace should start");
    assert_ne!(server.session.port, other_server.session.port);
    let second_file = ureq::get(&format!(
        "http://127.0.0.1:{}/api/contents/physics.txt",
        other_server.session.port
    ))
    .set(
        "Authorization",
        &format!("token {}", other_server.session.token),
    )
    .call();
    other_server.stop();
    assert_eq!(
        second_file
            .expect("second workspace file should be visible")
            .status(),
        200
    );
    assert!(
        jupyter::status_ready(&server.session),
        "opening another workspace must keep the first alive"
    );

    let url = jupyter::session_url(
        &server.session,
        &jupyter::lab_workspace(server.session.port),
        &relative_path,
    );
    let response = ureq::get(&url).call();

    // The token must actually be required: an unauthenticated request should be refused,
    // otherwise anything running locally could read the user's notebooks.
    let unauthenticated = ureq::get(&format!(
        "http://127.0.0.1:{}/api/contents",
        server.session.port
    ))
    .call();

    server.stop();

    let response = response.expect("the notebook page should load");
    assert_eq!(response.status(), 200, "notebook page did not return 200");

    match unauthenticated {
        Err(ureq::Error::Status(status, _)) => {
            assert!(
                status == 401 || status == 403,
                "expected an auth failure, got {status}"
            );
        }
        Err(other) => panic!("unexpected error on unauthenticated request: {other}"),
        Ok(_) => panic!("notebook API answered without a token — the session is unprotected"),
    }

    println!("SAGE NOTEBOOK OPENED SUCCESSFULLY");
    crate::runtime::wsl::ensure_owned(&paths.app_data_dir).expect("test runtime should be owned");
    crate::runtime::wsl::stop_checked().expect("runtime should stop");
    assert_eq!(crate::runtime::wsl::distro_running_state(), Some(false));
    let state = crate::state::AppState::new(
        crate::config::ConfigStore::new(&paths.app_data_dir),
        crate::state::AppPaths {
            app_data_dir: paths.app_data_dir.clone(),
            workspace_dir: paths.workspace_dir.clone(),
            package_search_dirs: Vec::new(),
        },
    );
    let home = crate::commands::setup_status(&state).expect("Home should refresh after stopping");
    assert!(home.environment_ready);
    assert_eq!(
        crate::runtime::wsl::distro_running_state(),
        Some(false),
        "observing must not restart WSL"
    );
    assert!(
        paths.workspace_dir.join(&relative_path).is_file(),
        "stopping must preserve notebooks"
    );
    let before: std::collections::BTreeSet<_> = std::fs::read_dir(&paths.workspace_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    // Home's launcher is rooted at the *default* SageDock folder, not whichever course was
    // opened last, and addresses JupyterLab's landing route rather than the file browser.
    // Both are checked against the running server, not just asserted about the string.
    let session = state
        .session_for(&paths.workspace_dir)
        .expect("Home must open JupyterLab without creating or choosing a notebook");
    let landing = jupyter::session_url(&session, &jupyter::lab_workspace(session.port), "");
    assert!(
        landing.contains("/lab/workspaces/") && !landing.contains("/tree"),
        "Home must open the launcher, not the file browser: {landing}"
    );
    let response = ureq::get(&landing)
        .call()
        .expect("the JupyterLab landing page should load");
    assert_eq!(response.status(), 200);
    let body = response.into_string().unwrap_or_default();
    assert!(
        body.contains("jupyter-config-data") || body.to_lowercase().contains("jupyterlab"),
        "the landing route did not serve the JupyterLab application itself"
    );
    let reused = state
        .session_for(&paths.workspace_dir)
        .expect("reopening should reuse the service");
    assert_eq!(
        session.port, reused.port,
        "reopening must reuse the existing service"
    );
    let after: std::collections::BTreeSet<_> = std::fs::read_dir(&paths.workspace_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        before, after,
        "opening JupyterLab must not create files or template folders"
    );
    assert_eq!(state.live_server_count(), 1);
    state.shutdown_jupyter();
    crate::runtime::provision::verify_installation()
        .expect("both kernels must execute after restarting");
    println!("MULTI-WORKSPACE AND STOP/RESTART VERIFIED");
}

/// Confirms the installed environment really is the one the image promised, using the same
/// contract the app relies on at runtime.
#[test]
#[ignore]
fn installed_environment_reports_sage_and_kernels() {
    use crate::runtime::{contract, wsl};

    require_qa_environment();

    let verify = wsl::run_as(wsl::LINUX_USER, &[contract::VERIFY]).expect("verify should run");
    println!("{}", verify.combined_output);

    assert!(verify.combined_output.contains("2^6 * 3 * 643"));
    assert!(verify.combined_output.contains("PYTHON_IMPORTS=ok"));

    let kernels = verify
        .combined_output
        .lines()
        .find(|line| line.starts_with("KERNELS="))
        .expect("verify output should list kernels");
    assert!(
        kernels.contains("sagemath"),
        "no sagemath kernel: {kernels}"
    );
    assert!(kernels.contains("python3"), "no python3 kernel: {kernels}");
}

/// A path containing characters a shell would treat as syntax must survive intact — the
/// bug that `--exec` fixes.
#[test]
#[ignore]
fn workspace_paths_with_shell_characters_survive() {
    use crate::runtime::wsl;

    require_qa_environment();

    let awkward = "/tmp/sagedock test $HOME 'quoted'";
    let result =
        wsl::run_as(wsl::LINUX_USER, &["printf", "%s", awkward]).expect("printf should run");

    assert_eq!(
        result.combined_output.trim_end(),
        awkward,
        "the path was rewritten in transit"
    );
}

/// Real compiler installation, inside the isolated QA runtime.
///
/// This is the only test that proves the claim the Tools page makes. The unit tests cover
/// the decision logic against synthetic component results; nothing but this actually
/// installs a compiler, compiles a program with it, and runs the result.
#[test]
#[ignore]
fn compilers_install_and_really_build_a_program() {
    use crate::runtime::wsl;
    use crate::scientific::{self, ReportSource, Tool, ToolState};

    require_qa_environment();
    let data = target_dir("SAGEDOCK_TEST_APPDATA", "appdata");

    let started = Instant::now();
    let report = scientific::install(Tool::Toolkit, &data, &mut |progress| {
        println!(
            "[{:>4}s] {:<24} {}",
            started.elapsed().as_secs(),
            progress.title,
            progress.detail.clone().unwrap_or_default()
        );
    })
    .expect("the full toolkit should install");

    let cpp = report
        .tools
        .iter()
        .find(|t| t.id == Tool::Cpp)
        .expect("the report must cover the tool that was installed");
    assert_eq!(
        cpp.state,
        ToolState::Installed,
        "components: {:?}",
        cpp.components
    );
    // Both languages, each proved by compiling and running something.
    assert_eq!(cpp.components.len(), 2);
    assert!(
        cpp.components.iter().all(|c| c.works),
        "a compiler that cannot build and run a program is not installed: {:?}",
        cpp.components
    );
    assert!(
        cpp.version.is_some(),
        "an installed compiler must report its version"
    );

    assert!(
        report
            .tools
            .iter()
            .filter(|t| !t.components.is_empty())
            .all(|t| t.state == ToolState::Installed),
        "every compiler and build tool must pass: {:?}",
        report
    );
    // Damage a versioned compiler binary and a C header in this disposable QA runtime.
    // Meta-package-only installs report success without restoring either one.
    wsl::ensure_owned(&data).unwrap();
    let damage = wsl::run_as("root", &["bash", "-c", "set -eu; binary=$(readlink -f /usr/bin/gfortran); case \"$binary\" in /usr/bin/*gfortran*) rm -- \"$binary\";; *) exit 64;; esac; rm -- /usr/include/stdio.h"]).unwrap();
    assert!(damage.success, "could not arrange damaged-package test");
    let broken = scientific::verify_now(&data).unwrap();
    assert!(broken
        .tools
        .iter()
        .any(|t| t.id == Tool::Toolkit && t.state == ToolState::NeedsRepair));
    let repaired = scientific::install(Tool::Toolkit, &data, &mut |_| {})
        .expect("repair must restore compiler payloads and headers");
    assert!(
        repaired
            .tools
            .iter()
            .any(|t| t.id == Tool::Toolkit && t.state == ToolState::Installed),
        "{:?}",
        repaired
    );

    // Installing again must not repeat completed work. A second apt transaction would take
    // far longer than re-probing does.
    let again = Instant::now();
    let mut stages = Vec::new();
    scientific::install(Tool::Toolkit, &data, &mut |p| stages.push(p.stage))
        .expect("installing an already-installed tool should succeed");
    assert!(!stages.contains(&scientific::ToolStage::Installing));
    assert_eq!(
        scientific::report(&data, false).source,
        ReportSource::Cached,
        "passive reads must not compile even when WSL is running"
    );
    println!("re-install settled in {}s", again.elapsed().as_secs());

    // Stopping, then reading the status, must leave the environment stopped: Home reads
    // this on every visit and must never undo a deliberate Stop.
    wsl::ensure_owned(&data).expect("test runtime should be owned");
    wsl::stop_checked().expect("runtime should stop");
    assert_eq!(wsl::distro_running_state(), Some(false));

    let cached = scientific::report(&data, false);
    assert_eq!(
        cached.source,
        ReportSource::Cached,
        "a stopped environment must yield the remembered result, not a fresh probe"
    );
    assert_eq!(
        wsl::distro_running_state(),
        Some(false),
        "reading the tool status restarted the environment"
    );
    assert!(
        cached
            .tools
            .iter()
            .any(|t| t.id == Tool::Cpp && t.state == ToolState::Installed),
        "the cached report lost the compiler it had verified"
    );
    println!("COMPILER INSTALL AND CACHED STATUS VERIFIED");
}

/// Sanity check that the shipped image's own path helper agrees with what WSL sees.
#[test]
#[ignore]
fn windows_workspace_is_visible_inside_linux() {
    use crate::runtime::wsl;
    require_qa_environment();

    let workspace = scratch_dir("visible");
    std::fs::write(workspace.join("marker.txt"), b"hello from windows").unwrap();

    let linux_path = wsl::windows_path_to_wsl(&workspace).expect("path should translate");
    let marker = format!("{linux_path}/marker.txt");

    let result = wsl::run_as(wsl::LINUX_USER, &["cat", &marker]).expect("cat should run");
    assert!(
        result.combined_output.contains("hello from windows"),
        "could not read the Windows workspace from Linux: {}",
        result.combined_output
    );
}

/// Exercises the exact webview command script against the shipped JupyterLab, not a mock.
#[test]
#[ignore]
fn jupyter_launcher_preserves_unsaved_notebooks() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    require_qa_environment();
    let root = scratch_dir("jupyter-ui");
    let first = create_notebook(&root, NotebookKind::Sage).unwrap();
    let second = create_notebook(&root, NotebookKind::Python).unwrap();
    let mut server = jupyter::start_ready(&root).unwrap();
    let port = server.session.port;
    let payload = serde_json::json!({
        "url": jupyter::session_url(&server.session, &jupyter::lab_workspace(port), ""),
        "home": crate::desktop::viewer_script(port, ""),
        "first": crate::desktop::viewer_script(port, &first),
        "second": crate::desktop::viewer_script(port, &second),
        "firstPath": first, "secondPath": second,
    });
    let mut child = Command::new("node")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/runtime/jupyter.mjs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Node and Edge are required for real Jupyter UI verification");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    server.stop();
    assert!(
        output.status.success(),
        "Jupyter UI test failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
}
