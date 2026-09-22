//! The single place in this codebase that talks to `wsl.exe`.
//!
//! Every WSL interaction funnels through here so that command construction, elevation,
//! output decoding, and distro naming live in one auditable spot rather than being
//! scattered across UI or feature code.
//!
//! Two rules this module exists to enforce:
//!
//! 1. **Programs inside the distro are started with `--exec`, never `--`.** This was
//!    verified on WSL 2.7 rather than assumed: with `--`, WSL rejoins the arguments into
//!    one command line for the Linux shell, which expanded `$HOME` and swallowed
//!    backslashes in a test argument. With `--exec`, every argument arrived byte-for-byte.
//!    Using `--` means any path or script containing `$`, a quote, or a backslash is
//!    silently rewritten, or interpreted as shell syntax.
//! 2. **No parsing of localized text to make decisions.** `wsl.exe` output is localized and
//!    has no machine-readable mode, so decisions use exit codes, and distro enumeration
//!    reads the registry instead of `wsl --list`.

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult, ErrorSeverity};
use crate::system::process::{run_hidden, ProcessResult};

/// The distro SageDock creates and manages. Deliberately distinct from any Ubuntu the user
/// installed themselves, so SageDock can rebuild its environment without touching theirs.
pub const DISTRO_NAME: &str = "SageDock";

/// Test builds alone can use a throwaway distribution. Production always uses SageDock.
pub fn distro_name() -> &'static str {
    #[cfg(test)]
    {
        static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        NAME.get_or_init(|| match std::env::var("SAGEDOCK_QA_DISTRO") {
            Ok(name) => {
                assert!(
                    name.starts_with("SageDockQA-")
                        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                    "Unsafe QA distro name"
                );
                name
            }
            Err(_) => DISTRO_NAME.into(),
        })
    }
    #[cfg(not(test))]
    {
        DISTRO_NAME
    }
}

/// Non-root Linux user that notebooks run as.
pub const LINUX_USER: &str = "sage";

const LXSS_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Lxss";

// --- availability ----------------------------------------------------------------------

/// Whether `wsl.exe` responds successfully. Uses only the exit code, see module docs.
///
/// Deliberately *not* proof that WSL 2 can run: see `vm_platform_present`.
pub fn is_available() -> bool {
    matches!(
        run_hidden("wsl.exe", &["--status"]),
        Ok(Some(ProcessResult { success: true, .. }))
    )
}

/// Whether the Windows Host Compute Service, the component WSL 2 uses to build its
/// lightweight virtual machine, exists on this system.
///
/// `wsl.exe --status` is not a substitute, and this function exists because trusting it
/// cost a real installation: on a PC where the Windows features were switched on but had
/// not taken effect yet, `--status` answered successfully, setup concluded Windows was
/// ready, skipped the whole components-and-restart step, and then died at `--import`,
/// the first operation that actually needs a virtual machine, with
/// `HCS_E_SERVICE_NOT_AVAILABLE` and an exit code of `-1` that said nothing.
///
/// `vmcompute` is registered by the `VirtualMachinePlatform` feature, so its absence is a
/// direct answer to the question setup actually needs answered. Only the exit code is
/// read: `sc.exe` fails with 1060 when a service does not exist.
pub fn vm_platform_present() -> bool {
    matches!(
        run_hidden("sc.exe", &["query", "vmcompute"]),
        Ok(Some(ProcessResult { success: true, .. }))
    )
}

/// Error code for "Windows is ready on paper, but needs a restart to mean it".
pub const RESTART_REQUIRED_CODE: &str = "WINDOWS_RESTART_REQUIRED";

/// Windows' own identifiers for "the virtual machine platform isn't running".
///
/// This is the one place that looks at `wsl.exe` output, and it is a deliberate exception
/// to the no-text-parsing rule in the module docs: these are stable error *identifiers*,
/// not the localized prose around them, and the process exit code for this failure is
/// `-1`, which distinguishes nothing. Matching is kept narrow on purpose, a generic
/// `CreateVm` failure (out of disk, for instance) must not be reported as "please
/// restart", which would send the user round a loop that could never fix it.
const RESTART_SIGNATURES: &[&str] = &[
    "HCS_E_SERVICE_NOT_AVAILABLE",
    "HCS_E_HYPERV_NOT_INSTALLED",
    "ERROR_HV_NOT_AVAILABLE",
    "0X80370102",
];

/// Whether failure output carries one of Windows' "needs a restart" identifiers.
pub fn output_means_restart_required(output: &str) -> bool {
    let upper = output.to_ascii_uppercase();
    RESTART_SIGNATURES
        .iter()
        .any(|signature| upper.contains(signature))
}

fn restart_required_error() -> AppError {
    AppError::new(
        "wsl",
        RESTART_REQUIRED_CODE,
        "Windows needs to restart before SageMath can be installed",
        "Windows has switched on the features SageDock needs, but they don't take effect until your computer restarts. Save your work in other apps, restart Windows, then reopen SageDock and choose Continue setup. Nothing on your PC was changed.",
    )
    .with_severity(ErrorSeverity::Warning)
}

/// Names of all registered WSL distros, read from the registry rather than parsed from
/// `wsl --list` (which is localized and pads output with UTF-16 and a default marker).
pub fn list_distros() -> Vec<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let Ok(lxss) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(LXSS_KEY) else {
        return Vec::new();
    };

    lxss.enum_keys()
        .filter_map(|name| name.ok())
        .filter_map(|name| lxss.open_subkey(&name).ok())
        .filter_map(|key| key.get_value::<String, _>("DistributionName").ok())
        .collect()
}

pub fn distro_exists(name: &str) -> bool {
    list_distros().iter().any(|d| d == name)
}

/// Whether SageDock's environment is running right now.
///
/// `--quiet` prints bare distribution names: no header, no decoration, no localized prose.
/// That is what makes comparing it safe under the no-text-parsing rule above, a
/// distribution name is neither localized nor prose, and it is the same string this module
/// passed to `--import`. The exit code cannot answer this on its own, since `wsl.exe`
/// reports failure both when nothing is running and when something went wrong.
///
/// Deliberately does not start anything: asking whether the environment is running must
/// never be the reason it starts.
pub fn distro_is_running() -> bool {
    distro_running_state().unwrap_or(false)
}

pub fn distro_running_state() -> Option<bool> {
    let Ok(Some(result)) = run_hidden("wsl.exe", &["--list", "--running", "--quiet"]) else {
        return None;
    };
    if !result.success {
        return None;
    }
    // UTF-16 output decodes with stray NULs on some Windows builds; trim them with the
    // whitespace rather than assuming a clean line.
    Some(
        result.combined_output.lines().any(|line| {
            line.trim_matches(|c: char| c.is_whitespace() || c == '\0') == distro_name()
        }),
    )
}

// --- elevated install --------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum ElevatedOutcome {
    /// Every step succeeded outright; WSL should be usable without restarting.
    Completed,
    /// A step reported success-pending-restart. Windows genuinely needs a reboot.
    RestartRequired,
    /// The user dismissed or denied the Windows permission prompt.
    DeclinedByUser,
}

/// What the supervisor has to say. Delivered through one callback rather than two so the
/// caller can forward both with a single mutable borrow of its own progress sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevationEvent {
    /// Windows is showing its permission prompt and nothing will progress until the user
    /// answers it. The prompt can appear behind the app window, so the UI says where to
    /// look rather than leaving the student staring at a spinner.
    AwaitingConsent,
    /// Permission was granted and the elevated helper is doing the work.
    HelperRunning,
    /// The helper is still alive. Liveness only, never progress.
    Heartbeat,
}

/// What the elevated helper reported. Parsed from the result file it writes.
#[derive(Debug, Clone, serde::Deserialize)]
struct ElevatedReport {
    /// Echoes the operation id, so a result left behind by an earlier run can never be
    /// mistaken for this one's.
    operation_id: String,
    /// "completed", "restart_required", or "failed".
    status: String,
    #[serde(default)]
    exit_code: i32,
    #[serde(default)]
    detail: String,
}

/// How long to keep waiting once the helper process is **gone** and no result appeared.
/// Short, because a vanished process is a settled fact, this only covers the moment
/// between the process exiting and its file becoming readable.
const RESULT_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// How long to wait for the user to answer the permission prompt before giving up on it.
/// Generous: a student may not have noticed the prompt, and the honest failure here is
/// "nobody answered", not "installation failed".
const CONSENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Enables the Windows features WSL needs and installs WSL, behind one UAC prompt.
///
/// This is the only operation SageDock performs with administrator rights, and it runs as a
/// separate short-lived elevated process, the app itself stays unelevated.
///
/// ## Why this supervises rather than blocks
///
/// The previous implementation ran `Start-Process -Verb RunAs -Wait` inside a hidden shell
/// and simply waited up to thirty minutes for an exit code. That produced the reported
/// stall: from the moment the UAC prompt appeared, the app emitted nothing at all, so an
/// unanswered prompt and a running installation looked identical, a frozen window. Worse,
/// the timeout killed the *outer* shell, which an unelevated process cannot use to stop the
/// elevated `dism` it started; the installation carried on invisibly while the app reported
/// that it had stopped.
///
/// So the launcher no longer waits. It returns as soon as consent is settled, handing back
/// the helper's process id, and this function supervises from there:
///
/// - Before the id arrives, the user is answering the prompt, `AwaitingConsent`.
/// - After it arrives, Windows is working, `HelperRunning`, with liveness checked against
///   the real process rather than inferred from silence. `dism` is routinely quiet for
///   minutes at a time, so quiet output is never treated as a hang.
/// - Nothing is ever killed. A helper that outlives our patience is reported as still
///   running, because that is what is true.
///
/// ## The result channel
///
/// An exit code alone cannot distinguish "the helper never started" from "dism failed with
/// code 1", and `Start-Process -PassThru`'s `ExitCode` is null often enough that `exit
/// $p.ExitCode` could report success for a run that never happened. The helper therefore
/// writes a small JSON report to a file named for this operation, and this function
/// accepts it only if the embedded operation id matches.
///
/// That file lives in SageDock's own application data directory, not a world-writable
/// temporary folder, and the code the helper runs is still passed immutably on its command
/// line rather than read from a script on disk. To be clear about what this does and does
/// not defend against: it prevents a stale or concurrent run's result being mistaken for
/// this one's. It is not a defence against an attacker who can already write to the user's
/// own application data, who would have easier targets there anyway.
pub fn install_wsl_elevated(
    app_data_dir: &Path,
    operation_id: &str,
    on_event: &mut dyn FnMut(ElevationEvent),
) -> AppResult<ElevatedOutcome> {
    use base64::Engine;

    let result_path = app_data_dir.join(format!("elevated-{operation_id}.json"));
    // A leftover file from a previous attempt would be read as this attempt's answer.
    let _ = std::fs::remove_file(&result_path);
    std::fs::create_dir_all(app_data_dir).map_err(|e| install_error(e.to_string()))?;

    let script = elevated_script(operation_id, &result_path);
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);

    // No `-Wait`: consent is the only thing this launcher waits for. It prints the helper's
    // process id so the supervisor below can watch the real process.
    let launcher = format!(
        r#"try {{
        $p = Start-Process -FilePath "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -ArgumentList '-NoProfile','-NonInteractive','-EncodedCommand','{encoded}' -Verb RunAs -WindowStyle Hidden -PassThru -ErrorAction Stop
        Write-Output "SAGEDOCK_PID=$($p.Id)"
        exit 0
    }} catch {{ if ($_.Exception.NativeErrorCode -eq 1223) {{ exit 1223 }}; Write-Output "SAGEDOCK_LAUNCH_ERROR=$($_.Exception.Message)"; exit 1 }}"#
    );

    on_event(ElevationEvent::AwaitingConsent);
    let launch = crate::system::process::run_hidden_timeout(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", &launcher],
        CONSENT_TIMEOUT,
    )
    .map_err(|e| {
        // A timeout here means the prompt went unanswered. Nothing was installed and
        // nothing was left running, so this is a plain "try again", not a failure state.
        install_error(format!("waiting for permission: {e}")).with_severity(ErrorSeverity::Warning)
    })?
    .ok_or_else(|| install_error("Windows PowerShell is unavailable".into()))?;

    if launch.exit_code == Some(1223) {
        let _ = std::fs::remove_file(&result_path);
        return Ok(ElevatedOutcome::DeclinedByUser);
    }
    let Some(pid) = parse_helper_pid(&launch.combined_output) else {
        let _ = std::fs::remove_file(&result_path);
        // The helper never started. Distinguished from a helper that started and failed,
        // because the two need different advice.
        return Err(install_error(format!(
            "the elevated helper did not start (exit {:?}): {}",
            launch.exit_code,
            launch.combined_output.trim(),
        )));
    };

    on_event(ElevationEvent::HelperRunning);
    let outcome = supervise_helper(pid, &result_path, operation_id, on_event);
    let _ = std::fs::remove_file(&result_path);
    outcome
}

/// Watches the elevated helper until it reports, disappears, or is still going when asked
/// about. Never kills it: this process is unelevated and could not stop it anyway, and
/// pretending otherwise is how the old code came to report a stopped installation that was
/// in fact still running.
fn supervise_helper(
    pid: u32,
    result_path: &Path,
    operation_id: &str,
    on_event: &mut dyn FnMut(ElevationEvent),
) -> AppResult<ElevatedOutcome> {
    let mut gone_since: Option<std::time::Instant> = None;
    loop {
        if let Some(report) = read_report(result_path, operation_id) {
            return classify_report(&report);
        }
        if crate::setup::process_is_running(pid) {
            gone_since = None;
        } else {
            // The process ended. Give the filesystem a moment for its result to land
            // before concluding it wrote nothing.
            let since = gone_since.get_or_insert_with(std::time::Instant::now);
            if since.elapsed() >= RESULT_GRACE {
                return Err(install_error(format!(
                    "the elevated helper (pid {pid}) ended without reporting a result",
                )));
            }
        }
        on_event(ElevationEvent::Heartbeat);
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

/// Reads the helper's report, ignoring one written for a different operation.
fn read_report(path: &Path, operation_id: &str) -> Option<ElevatedReport> {
    let raw = std::fs::read_to_string(path).ok()?;
    let report: ElevatedReport = serde_json::from_str(&raw).ok()?;
    (report.operation_id == operation_id).then_some(report)
}

/// Distinguishes completion, required reboot, and actual failure.
fn classify_report(report: &ElevatedReport) -> AppResult<ElevatedOutcome> {
    match report.status.as_str() {
        "completed" => Ok(ElevatedOutcome::Completed),
        "restart_required" => Ok(ElevatedOutcome::RestartRequired),
        _ => Err(install_error(format!(
            "Windows setup reported {} (exit {}): {}",
            report.status, report.exit_code, report.detail,
        ))),
    }
}

/// Pulls the helper's process id out of the launcher's output.
fn parse_helper_pid(output: &str) -> Option<u32> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("SAGEDOCK_PID="))
        .and_then(|value| value.trim().parse().ok())
        .filter(|&pid| pid != 0)
}

/// The code the elevated helper runs.
///
/// `dism` is invoked per feature so a failure names the feature that failed, and the whole
/// thing is wrapped so the result file is written on every path, including an unexpected
/// exception. A helper that dies without writing is detectable (the supervisor sees the
/// process disappear with no report) but not diagnosable, so it is worth the `finally`.
fn elevated_script(operation_id: &str, result_path: &Path) -> String {
    // Both values are produced by SageDock, not by the user: the id is hex from
    // `new_operation_id` and the path is built from the app's own data directory. They are
    // still single-quoted with embedded quotes doubled, so neither can close its literal
    // and run as code.
    let id = ps_single_quoted(operation_id);
    let path = ps_single_quoted(&result_path.to_string_lossy());
    format!(
        r#"
$ErrorActionPreference = 'Stop'
$resultPath = {path}
$report = @{{ operation_id = {id}; status = 'failed'; exit_code = 0; detail = 'the helper ended unexpectedly' }}
try {{
    $restart = $false
    foreach ($feature in @('Microsoft-Windows-Subsystem-Linux', 'VirtualMachinePlatform')) {{
        & "$env:SystemRoot\System32\dism.exe" /online /enable-feature /featurename:$feature /all /norestart *> $null
        $code = $LASTEXITCODE
        if ($code -eq 3010) {{ $restart = $true }}
        elseif ($code -ne 0) {{
            $report.status = 'failed'
            $report.exit_code = $code
            $report.detail = "enabling $feature failed"
            throw "dism $feature exit $code"
        }}
    }}
    if ($restart) {{
        $report.status = 'restart_required'
        $report.exit_code = 3010
        $report.detail = 'a Windows feature needs a restart to finish'
    }} else {{
        & "$env:SystemRoot\System32\wsl.exe" --install --no-distribution *> $null
        $code = $LASTEXITCODE
        if ($code -eq 0) {{
            $report.status = 'completed'
            $report.detail = 'components enabled'
        }} elseif ($code -eq 3010) {{
            $report.status = 'restart_required'
            $report.exit_code = 3010
            $report.detail = 'the computing component needs a restart to finish'
        }} else {{
            $report.status = 'failed'
            $report.exit_code = $code
            $report.detail = 'installing the computing component failed'
        }}
    }}
}} catch {{
    if ($report.detail -eq 'the helper ended unexpectedly') {{ $report.detail = $_.Exception.Message }}
}} finally {{
    try {{ ConvertTo-Json $report -Compress | Set-Content -LiteralPath $resultPath -Encoding UTF8 }} catch {{ }}
}}
exit 0
"#
    )
}

/// Wraps a value as a PowerShell single-quoted literal, doubling any embedded quote.
/// Inside single quotes PowerShell performs no expansion at all, so a doubled quote is the
/// only escape that matters.
fn ps_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn install_error(details: String) -> AppError {
    AppError::new("wsl", "WSL_INSTALL_FAILED", "Windows couldn't finish preparing your computer",
        "Your saved notebooks are safe. Check your internet connection and finish any pending Windows updates, then try setup again. Windows may need to download its computing component.")
        .with_technical_details(details)
}

// --- distro lifecycle ----------------------------------------------------------------------

/// Imports a runtime image archive (`.tar`, `.tar.gz` or `.tar.xz`) as the SageDock distro.
pub fn import_distro(install_dir: &Path, archive: &Path) -> AppResult<()> {
    std::fs::create_dir_all(install_dir).map_err(|err| {
        AppError::new(
            "wsl",
            "RUNTIME_DIR_CREATE_FAILED",
            "SageDock couldn't create its environment folder",
            "SageDock wasn't able to create the folder that holds SageMath. Check that you have free disk space and try again.",
        )
        .with_technical_details(err.to_string())
    })?;

    let install_dir = install_dir.to_string_lossy().to_string();
    let archive = archive.to_string_lossy().to_string();

    run_checked(
        &[
            "--import",
            distro_name(),
            &install_dir,
            &archive,
            "--version",
            "2",
        ],
        "RUNTIME_IMPORT_FAILED",
        "SageDock couldn't install SageMath",
        "The SageMath package couldn't be installed. Your notebooks and files were not affected.",
    )
    .map_err(|err| {
        // A machine whose Windows features are on but not yet active fails here, and the
        // package is blameless. Telling the user their SageMath file is broken would send
        // them hunting for a new download instead of restarting.
        if err
            .technical_details
            .as_deref()
            .is_some_and(output_means_restart_required)
        {
            restart_required_error()
                .with_technical_details(err.technical_details.unwrap_or_default())
        } else {
            err
        }
    })?;

    tracing::info!(target: "runtime", "imported runtime image");
    Ok(())
}

/// Removes the SageDock distro.
///
/// Destroys the environment but never user files, which live on the Windows side by
/// design. Setup only calls this for a distro it created moments earlier from a file that
/// turned out not to be a SageDock image.
pub fn unregister_distro() -> AppResult<()> {
    run_checked(
        &["--unregister", distro_name()],
        "RUNTIME_UNREGISTER_FAILED",
        "SageDock couldn't remove its computing environment",
        "The computing environment could not be removed. Restarting your computer and trying again usually resolves this.",
    )?;
    Ok(())
}

// --- running programs inside the distro ------------------------------------------------------

/// Stops SageDock's Linux environment, releasing the memory WSL holds for it.
///
/// Deliberately `--terminate <distro>` and never `wsl --shutdown`: the latter stops every
/// distro on the machine, which would kill unrelated work belonging to someone who uses
/// WSL for their own projects. Best-effort, a failure here only means it wasn't running.
pub fn terminate_distro() {
    match run_hidden("wsl.exe", &["--terminate", distro_name()]) {
        Ok(Some(result)) => {
            tracing::info!(target: "runtime", success = result.success, "environment stopped")
        }
        _ => tracing::debug!(target: "runtime", "environment was not running"),
    }
}

pub fn stop_checked() -> AppResult<()> {
    run_checked(&["--terminate", distro_name()], "STOP_FAILED", "The computing environment couldn't stop",
        "Your saved notebooks are safe. Wait a moment and try again before making changes to the environment.")?;
    Ok(())
}

pub fn export_backup(path: &Path) -> AppResult<()> {
    run_checked(&["--export", distro_name(), &path.to_string_lossy()], "BACKUP_FAILED", "SageDock couldn't back up its environment",
        "No environment has been removed. Free more storage and try again. Your notebooks are safe.")?;
    Ok(())
}

pub fn ensure_owned(data: &Path) -> AppResult<()> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let expected = default_install_dir(data).canonicalize().ok();
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(LXSS_KEY).ok();
    if let Some(key) = key {
        for name in key.enum_keys().filter_map(Result::ok) {
            if let Ok(entry) = key.open_subkey(name) {
                if entry
                    .get_value::<String, _>("DistributionName")
                    .ok()
                    .as_deref()
                    == Some(distro_name())
                {
                    let actual = entry
                        .get_value::<String, _>("BasePath")
                        .ok()
                        .and_then(|p| PathBuf::from(p).canonicalize().ok());
                    if expected.is_some() && actual == expected {
                        return Ok(());
                    }
                }
            }
        }
    }
    Err(AppError::new("runtime", "RUNTIME_OWNERSHIP_UNKNOWN", "SageDock can't safely replace this environment",
        "An environment with this name exists in another location. It has been left untouched to protect its files. Export a diagnostic report for support."))
}

/// Builds the `wsl.exe` argument list for running a program inside the SageDock distro.
pub fn exec_args<'a>(user: &'a str, program_and_args: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec!["-d", distro_name(), "-u", user, "--exec"];
    args.extend_from_slice(program_and_args);
    args
}

/// Runs a program inside the SageDock distro as `user`, with every argument passed verbatim.
pub fn run_as(user: &str, program_and_args: &[&str]) -> AppResult<ProcessResult> {
    run_as_timeout(user, program_and_args, 60)
}

pub fn run_as_timeout(
    user: &str,
    program_and_args: &[&str],
    seconds: u64,
) -> AppResult<ProcessResult> {
    crate::system::process::run_hidden_timeout("wsl.exe", &exec_args(user, program_and_args), std::time::Duration::from_secs(seconds))
        .map_err(|err| {
            AppError::new(
                "runtime",
                "RUNTIME_COMMAND_SPAWN_FAILED",
                "SageDock couldn't reach its computing environment",
                "SageDock wasn't able to communicate with its computing environment. Repairing it usually fixes this.",
            )
            .with_technical_details(err.to_string())
        })?
        .ok_or_else(|| {
            AppError::new(
                "runtime",
                "WSL_MISSING",
                "SageDock's computing environment could not be found",
                "The Windows component SageDock uses to run SageMath isn't available. This can usually be repaired without affecting your notebooks.",
            )
        })
}

// --- path translation ---------------------------------------------------------------------------

/// Converts an absolute Windows path to the `/mnt/<drive>/...` path WSL exposes it at.
///
/// Rejects anything that isn't a plain absolute path with a drive letter rather than
/// guessing, so a malformed path can't silently become a different location inside Linux.
/// Does not assume any particular drive letter.
pub fn windows_path_to_wsl(path: &Path) -> AppResult<String> {
    let path_str = path.to_string_lossy().replace('\\', "/");
    let bytes = path_str.as_bytes();

    let looks_absolute =
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/';

    if !looks_absolute {
        return Err(AppError::new(
            "runtime",
            "PATH_NOT_TRANSLATABLE",
            "SageDock couldn't use that folder",
            "SageDock can only work with folders stored on this computer's drives. Choosing a folder inside your Documents usually works.",
        )
        .with_technical_details(format!("path: {}", path.display())));
    }

    let drive = (bytes[0] as char).to_ascii_lowercase();
    Ok(format!("/mnt/{drive}{}", &path_str[2..]))
}

/// Where SageDock stores the distro's virtual disk.
pub fn default_install_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("runtime").join(distro_name())
}

fn run_checked(args: &[&str], code: &str, title: &str, message: &str) -> AppResult<ProcessResult> {
    let result = crate::system::process::run_hidden_timeout(
        "wsl.exe",
        args,
        std::time::Duration::from_secs(1800),
    )
    .map_err(|err| {
        AppError::new("runtime", code, title, message).with_technical_details(err.to_string())
    })?
    .ok_or_else(|| {
        AppError::new("runtime", code, title, message)
            .with_technical_details("wsl.exe could not be found")
    })?;

    if !result.success {
        return Err(
            AppError::new("runtime", code, title, message).with_technical_details(format!(
                "wsl.exe {}\nexit_code: {:?}\n{}",
                args.join(" "),
                result.exit_code,
                result.combined_output
            )),
        );
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(status: &str, exit_code: i32) -> ElevatedReport {
        ElevatedReport {
            operation_id: "op1".into(),
            status: status.into(),
            exit_code,
            detail: "detail".into(),
        }
    }

    #[test]
    fn the_helpers_report_preserves_reboot_and_completion() {
        assert_eq!(
            classify_report(&report("completed", 0)).unwrap(),
            ElevatedOutcome::Completed
        );
        assert_eq!(
            classify_report(&report("restart_required", 3010)).unwrap(),
            ElevatedOutcome::RestartRequired
        );
    }

    #[test]
    fn any_status_other_than_success_is_a_failure_rather_than_a_default_to_completed() {
        // The old code mapped a missing exit code to failure but a null `ExitCode` from
        // `Start-Process -PassThru` to `exit 0`, reporting success for a run that never
        // happened. An unrecognised status must never fall through to success.
        for status in ["failed", "", "unknown", "Completed"] {
            assert_eq!(
                classify_report(&report(status, 1)).unwrap_err().code,
                "WSL_INSTALL_FAILED",
                "status {status:?} must not be read as success",
            );
        }
    }

    #[test]
    fn a_report_from_a_different_operation_is_refused() {
        let dir = std::env::temp_dir().join(format!("sagedock-wsl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("report.json");
        std::fs::write(
            &path,
            br#"{"operation_id":"other","status":"completed","exit_code":0,"detail":""}"#,
        )
        .unwrap();
        // A result left behind by an earlier run must not be read as this run's answer,
        // or a retry would instantly "succeed" without Windows having done anything.
        assert!(read_report(&path, "op1").is_none());
        assert!(read_report(&path, "other").is_some());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_unreadable_or_absent_report_is_simply_not_an_answer_yet() {
        let missing = std::env::temp_dir().join("sagedock-no-such-report.json");
        let _ = std::fs::remove_file(&missing);
        assert!(read_report(&missing, "op1").is_none());
    }

    #[test]
    fn the_helper_process_id_is_read_back_from_the_launcher_output() {
        assert_eq!(parse_helper_pid("SAGEDOCK_PID=4321\n"), Some(4321));
        // Real output carries other lines around it.
        assert_eq!(
            parse_helper_pid("noise\r\nSAGEDOCK_PID=99\r\nmore noise"),
            Some(99),
        );
    }

    #[test]
    fn a_launcher_that_reported_no_process_id_is_not_treated_as_a_started_helper() {
        assert_eq!(parse_helper_pid(""), None);
        assert_eq!(
            parse_helper_pid("SAGEDOCK_LAUNCH_ERROR=Access denied"),
            None
        );
        // Zero is not a usable process id, and treating it as one would make the
        // supervisor watch nothing and wait forever.
        assert_eq!(parse_helper_pid("SAGEDOCK_PID=0"), None);
        assert_eq!(parse_helper_pid("SAGEDOCK_PID=notanumber"), None);
    }

    #[test]
    fn a_single_quote_in_an_embedded_value_cannot_close_its_literal() {
        assert_eq!(ps_single_quoted("plain"), "'plain'");
        assert_eq!(ps_single_quoted("it's"), "'it''s'");
        // The shape that would otherwise end the string and start a new statement.
        assert_eq!(
            ps_single_quoted("'; Remove-Item C:\\ -Recurse; '"),
            "'''; Remove-Item C:\\ -Recurse; '''",
        );
    }

    #[test]
    fn the_elevated_script_embeds_the_operation_id_so_its_report_can_be_matched() {
        let script = elevated_script("abc123", Path::new(r"C:\data\elevated-abc123.json"));
        assert!(script.contains("'abc123'"));
        assert!(script.contains(r"C:\data\elevated-abc123.json"));
        // Every path through the script must leave a report behind, or a failure becomes
        // indistinguishable from a helper that never ran.
        assert!(script.contains("finally"));
        assert!(script.contains("restart_required"));
    }

    /// The exact failure seen on a test PC: Windows reported WSL as available, the import
    /// ran anyway, and this identifier was the only usable evidence in the output, the
    /// exit code was `-1`.
    #[test]
    fn the_host_compute_service_error_is_recognised_as_needing_a_restart() {
        let observed = "wsl.exe --import SageDock C:\\Users\\Admin\\AppData\\Roaming\\com.sagedock.desktop\\runtime\\SageDock \
                        \\\\?\\C:\\Program Files\\SageDock\\runtime\\sagedock-runtime-sage10.9-x64.tar.xz --version 2\n\
                        exit_code: Some(-1)\n\
                        The operation could not be started because a required feature is not installed.\n\
                        Error code: Wsl/Service/RegisterDistro/CreateVm/HCS/HCS_E_SERVICE_NOT_AVAILABLE";
        assert!(output_means_restart_required(observed));
    }

    /// Matching must stay narrow: a genuine import failure reported as "restart Windows"
    /// would send the user round a loop that can never succeed.
    #[test]
    fn ordinary_import_failures_are_not_mistaken_for_a_restart() {
        for output in [
            "exit_code: Some(1)\nThere is not enough space on the disk.",
            "exit_code: Some(1)\nWsl/Service/RegisterDistro/ERROR_FILE_NOT_FOUND",
            "exit_code: Some(-1)\nThe archive is corrupt.",
            "",
        ] {
            assert!(
                !output_means_restart_required(output),
                "false positive on {output:?}"
            );
        }
    }

    #[test]
    fn restart_signatures_are_matched_regardless_of_case() {
        assert!(output_means_restart_required("hcs_e_service_not_available"));
        assert!(output_means_restart_required("Error code: 0x80370102"));
    }

    /// Regression test for a verified bug: `--` routes arguments through the Linux shell,
    /// which rewrote `$HOME` and backslashes in a probe. Only `--exec` passes them verbatim.
    #[test]
    fn programs_are_launched_with_exec_not_the_shell_separator() {
        let args = exec_args(LINUX_USER, &["/opt/sagedock/bin/sagedock-verify"]);
        assert!(args.contains(&"--exec"));
        assert!(
            !args.contains(&"--"),
            "`--` re-parses arguments through the Linux shell"
        );
        assert_eq!(args.last(), Some(&"/opt/sagedock/bin/sagedock-verify"));
    }

    #[test]
    fn exec_args_keep_argument_boundaries() {
        let args = exec_args(LINUX_USER, &["printf", "%s", "a b", "$HOME"]);
        assert_eq!(&args[args.len() - 4..], &["printf", "%s", "a b", "$HOME"]);
    }

    #[test]
    fn translates_a_normal_windows_path() {
        let path = Path::new(r"C:\Users\someone\Documents\SageDock");
        assert_eq!(
            windows_path_to_wsl(path).unwrap(),
            "/mnt/c/Users/someone/Documents/SageDock"
        );
    }

    /// Drive letters must not be assumed: redirected Documents folders live on other drives.
    #[test]
    fn translates_a_non_c_drive() {
        assert_eq!(
            windows_path_to_wsl(Path::new(r"D:\Work\notebooks")).unwrap(),
            "/mnt/d/Work/notebooks"
        );
    }

    #[test]
    fn keeps_quotes_and_spaces_in_paths_intact() {
        assert_eq!(
            windows_path_to_wsl(Path::new(r"C:\Users\Jo O'Brien\Documents")).unwrap(),
            "/mnt/c/Users/Jo O'Brien/Documents"
        );
    }

    #[test]
    fn rejects_unc_paths() {
        assert!(windows_path_to_wsl(Path::new(r"\\server\share\file")).is_err());
    }

    #[test]
    fn rejects_relative_paths() {
        assert!(windows_path_to_wsl(Path::new(r"notebooks\a.ipynb")).is_err());
    }
}
