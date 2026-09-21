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
//!    silently rewritten — or interpreted as shell syntax.
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

/// Whether `wsl.exe` responds successfully. Uses only the exit code — see module docs.
///
/// Deliberately *not* proof that WSL 2 can run: see `vm_platform_present`.
pub fn is_available() -> bool {
    matches!(
        run_hidden("wsl.exe", &["--status"]),
        Ok(Some(ProcessResult { success: true, .. }))
    )
}

/// Whether the Windows Host Compute Service — the component WSL 2 uses to build its
/// lightweight virtual machine — exists on this system.
///
/// `wsl.exe --status` is not a substitute, and this function exists because trusting it
/// cost a real installation: on a PC where the Windows features were switched on but had
/// not taken effect yet, `--status` answered successfully, setup concluded Windows was
/// ready, skipped the whole components-and-restart step, and then died at `--import` —
/// the first operation that actually needs a virtual machine — with
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
/// `-1`, which distinguishes nothing. Matching is kept narrow on purpose — a generic
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
/// That is what makes comparing it safe under the no-text-parsing rule above — a
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

#[derive(Debug)]
pub enum ElevatedOutcome {
    /// Every step succeeded outright; WSL should be usable without restarting.
    Completed,
    /// A step reported success-pending-restart. Windows genuinely needs a reboot.
    RestartRequired,
    /// The user dismissed or denied the Windows permission prompt.
    DeclinedByUser,
}

/// Enables the Windows features WSL needs and installs WSL, behind one UAC prompt.
///
/// This is the only operation SageDock performs with administrator rights, and it runs as a
/// separate short-lived elevated process — the app itself stays unelevated.
///
/// The elevated script checks each step and returns one Windows exit code. It is passed
/// as UTF-16 EncodedCommand, avoiding executable scripts in writable temporary folders.
pub fn install_wsl_elevated() -> AppResult<ElevatedOutcome> {
    use base64::Engine;
    // The elevated process receives immutable code in its command line. No executable
    // script or result file is placed in a user-writable temporary directory.
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$restart = $false
foreach ($feature in @('Microsoft-Windows-Subsystem-Linux', 'VirtualMachinePlatform')) {
    & "$env:SystemRoot\System32\dism.exe" /online /enable-feature /featurename:$feature /all /norestart *> $null
    $code = $LASTEXITCODE
    if ($code -eq 3010) { $restart = $true }
    elseif ($code -ne 0) { exit $code }
}
if ($restart) { exit 3010 }
& "$env:SystemRoot\System32\wsl.exe" --install --no-distribution *> $null
exit $LASTEXITCODE
"#;
    let bytes: Vec<u8> = SCRIPT.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let launcher = format!(
        r#"try {{
        $p = Start-Process -FilePath "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -ArgumentList '-NoProfile','-NonInteractive','-EncodedCommand','{encoded}' -Verb RunAs -WindowStyle Hidden -Wait -PassThru -ErrorAction Stop
        exit $p.ExitCode
    }} catch {{ if ($_.Exception.NativeErrorCode -eq 1223) {{ exit 1223 }}; exit 1 }}"#
    );
    let result = crate::system::process::run_hidden_timeout(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", &launcher],
        std::time::Duration::from_secs(1800),
    )
    .map_err(|e| install_error(e.to_string()))?
    .ok_or_else(|| install_error("Windows PowerShell is unavailable".into()))?;
    classify_install_exit(result.exit_code)
}

/// Distinguishes completion, required reboot, denied elevation, and actual failure.
fn classify_install_exit(exit_code: Option<i32>) -> AppResult<ElevatedOutcome> {
    match exit_code {
        Some(0) => Ok(ElevatedOutcome::Completed),
        Some(3010) => Ok(ElevatedOutcome::RestartRequired),
        Some(1223) => Ok(ElevatedOutcome::DeclinedByUser),
        code => Err(install_error(format!("Windows setup exit: {code:?}"))),
    }
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
/// WSL for their own projects. Best-effort — a failure here only means it wasn't running.
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

    #[test]
    fn installation_exit_codes_preserve_reboot_and_cancellation() {
        assert!(matches!(
            classify_install_exit(Some(0)).unwrap(),
            ElevatedOutcome::Completed
        ));
        assert!(matches!(
            classify_install_exit(Some(3010)).unwrap(),
            ElevatedOutcome::RestartRequired
        ));
        assert!(matches!(
            classify_install_exit(Some(1223)).unwrap(),
            ElevatedOutcome::DeclinedByUser
        ));
    }

    #[test]
    fn failed_or_missing_installation_exit_is_not_success() {
        for code in [
            None,
            Some(1),
            Some(87),
            Some(-1),
            Some(0x800f0954u32 as i32),
        ] {
            assert_eq!(
                classify_install_exit(code).unwrap_err().code,
                "WSL_INSTALL_FAILED"
            );
        }
    }

    /// The exact failure seen on a test PC: Windows reported WSL as available, the import
    /// ran anyway, and this identifier was the only usable evidence in the output — the
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
