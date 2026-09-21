//! WSL detection: whether it's installed, which version distros default to, and
//! whether a SageDock-managed distro is already registered.
//!
//! `wsl.exe` has no machine-readable (`--json`) output mode, and its human-readable
//! text is localized, so none of these checks parse its stdout to decide anything —
//! only the process exit code is used for `wsl_available`. The version and distro
//! checks instead read the registry directly under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss`, where WSL stores per-distro
//! registration (`DistributionName`) and the default-version preference
//! (`DefaultVersion`). This is community-documented, widely-observed behavior rather
//! than an officially published API, so both reads are treated as best-effort: absence
//! or an unreadable value degrades to "unknown," never to an error.
//!
use winreg::enums::*;
use winreg::RegKey;

use crate::error::ErrorSeverity;
use crate::system::process::run_hidden;
use crate::system::types::CheckItem;

const LXSS_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Lxss";

// --- wsl_available -----------------------------------------------------------------

fn gather_available() -> Option<bool> {
    let result = run_hidden("wsl.exe", &["--status"]).ok()??;
    tracing::debug!(target: "wsl", exit_code = ?result.exit_code, "wsl.exe --status");
    Some(result.success)
}

fn interpret_available(available: Option<bool>) -> CheckItem {
    match available {
        Some(true) => CheckItem {
            id: "wsl_available",
            label: "Windows Subsystem for Linux".into(),
            severity: ErrorSeverity::Info,
            summary: "Windows Subsystem for Linux is installed.".into(),
            detail: None,
        },
        Some(false) | None => CheckItem {
            id: "wsl_available",
            label: "Windows Subsystem for Linux".into(),
            severity: ErrorSeverity::Info,
            summary: "Windows Subsystem for Linux isn't installed yet. SageDock can install it automatically when you set it up.".into(),
            detail: None,
        },
    }
}

pub fn check_available() -> CheckItem {
    interpret_available(gather_available())
}

// --- wsl_default_version ------------------------------------------------------------

fn gather_default_version() -> Option<u32> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(LXSS_KEY)
        .and_then(|key| key.get_value::<u32, _>("DefaultVersion"))
        .ok()
}

fn interpret_default_version(version: Option<u32>) -> CheckItem {
    match version {
        Some(2) => CheckItem {
            id: "wsl_default_version",
            label: "WSL version".into(),
            severity: ErrorSeverity::Info,
            summary: "New Linux environments default to WSL 2, which SageDock needs.".into(),
            detail: None,
        },
        Some(other) => CheckItem {
            id: "wsl_default_version",
            label: "WSL version".into(),
            severity: ErrorSeverity::Info,
            summary: "New Linux environments currently default to an older WSL version. SageDock will configure WSL 2 automatically during setup.".into(),
            detail: Some(format!("DefaultVersion={other}")),
        },
        None => CheckItem {
            id: "wsl_default_version",
            label: "WSL version".into(),
            severity: ErrorSeverity::Info,
            summary: "SageDock will confirm and configure WSL 2 automatically during setup.".into(),
            detail: None,
        },
    }
}

pub fn check_default_version() -> CheckItem {
    interpret_default_version(gather_default_version())
}

// --- sagedock_distro -----------------------------------------------------------------

fn gather_sagedock_distro() -> bool {
    crate::runtime::wsl::distro_exists(crate::runtime::wsl::distro_name())
}

fn interpret_sagedock_distro(found: bool) -> CheckItem {
    if found {
        CheckItem {
            id: "sagedock_distro",
            label: "SageDock computing environment".into(),
            severity: ErrorSeverity::Info,
            summary: "A SageDock computing environment was found.".into(),
            detail: None,
        }
    } else {
        CheckItem {
            id: "sagedock_distro",
            label: "SageDock computing environment".into(),
            severity: ErrorSeverity::Info,
            summary: "No SageDock computing environment has been set up yet.".into(),
            detail: None,
        }
    }
}

pub fn check_sagedock_distro() -> CheckItem {
    interpret_sagedock_distro(gather_sagedock_distro())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_installed_is_healthy() {
        assert_eq!(
            interpret_available(Some(true)).severity,
            ErrorSeverity::Info
        );
    }

    #[test]
    fn wsl_missing_is_informational_not_an_error() {
        let check = interpret_available(None);
        assert_eq!(check.severity, ErrorSeverity::Info);
        assert!(check.summary.contains("install it automatically"));
    }

    #[test]
    fn default_version_two_is_healthy() {
        assert_eq!(
            interpret_default_version(Some(2)).severity,
            ErrorSeverity::Info
        );
    }

    #[test]
    fn default_version_one_is_still_informational() {
        let check = interpret_default_version(Some(1));
        assert_eq!(check.severity, ErrorSeverity::Info);
        assert!(check.summary.contains("automatically"));
    }

    #[test]
    fn missing_distro_is_informational() {
        assert_eq!(
            interpret_sagedock_distro(false).severity,
            ErrorSeverity::Info
        );
    }

    #[test]
    fn found_distro_is_informational() {
        assert_eq!(
            interpret_sagedock_distro(true).severity,
            ErrorSeverity::Info
        );
    }
}
