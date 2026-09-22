//! Windows version check. WSL2 requires Windows 10 build 19041 (version 2004) or later.
//!
//! Reads the build number from the registry rather than `GetVersionEx`/`GetVersion`,
//! which lie about the OS version unless the calling process has an application
//! manifest declaring compatibility with each Windows release, a manifest maintenance
//! burden this avoids entirely.

use winreg::enums::*;
use winreg::RegKey;

use crate::error::ErrorSeverity;
use crate::system::types::CheckItem;

const MIN_BUILD_FOR_WSL2: u32 = 19041;
const REGISTRY_PATH: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

struct WindowsVersionInfo {
    build: Option<u32>,
    display_version: Option<String>,
    product_name: Option<String>,
}

fn gather() -> WindowsVersionInfo {
    let read = || -> std::io::Result<WindowsVersionInfo> {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let key = hklm.open_subkey_with_flags(REGISTRY_PATH, KEY_READ)?;
        let build: String = key.get_value("CurrentBuildNumber")?;
        let display_version: Option<String> = key
            .get_value("DisplayVersion")
            .or_else(|_| key.get_value("ReleaseId"))
            .ok();
        let product_name: Option<String> = key.get_value("ProductName").ok();

        Ok(WindowsVersionInfo {
            build: build.parse().ok(),
            display_version,
            product_name,
        })
    };

    read().unwrap_or(WindowsVersionInfo {
        build: None,
        display_version: None,
        product_name: None,
    })
}

fn interpret(info: WindowsVersionInfo) -> CheckItem {
    let detail = format!(
        "Product: {} | Build: {} | Version: {}",
        info.product_name.as_deref().unwrap_or("unknown"),
        info.build
            .map(|b| b.to_string())
            .unwrap_or_else(|| "unknown".into()),
        info.display_version.as_deref().unwrap_or("unknown"),
    );

    let Some(build) = info.build else {
        return CheckItem {
            id: "windows_version",
            label: "Windows version".into(),
            severity: ErrorSeverity::Warning,
            summary: "SageDock couldn't confirm which version of Windows you're running.".into(),
            detail: Some(detail),
        };
    };

    if build >= MIN_BUILD_FOR_WSL2 {
        CheckItem {
            id: "windows_version",
            label: "Windows version".into(),
            severity: ErrorSeverity::Info,
            summary: "Windows is ready.".into(),
            detail: Some(detail),
        }
    } else {
        CheckItem {
            id: "windows_version",
            label: "Windows version".into(),
            severity: ErrorSeverity::Error,
            summary: "Windows needs to be updated. SageDock needs a newer version of Windows to run SageMath.".into(),
            detail: Some(detail),
        }
    }
}

pub fn check() -> CheckItem {
    interpret(gather())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(build: Option<u32>) -> WindowsVersionInfo {
        WindowsVersionInfo {
            build,
            display_version: Some("23H2".into()),
            product_name: Some("Windows 11 Pro".into()),
        }
    }

    #[test]
    fn modern_build_is_healthy() {
        let check = interpret(info(Some(22631)));
        assert_eq!(check.severity, ErrorSeverity::Info);
    }

    #[test]
    fn minimum_supported_build_is_healthy() {
        let check = interpret(info(Some(MIN_BUILD_FOR_WSL2)));
        assert_eq!(check.severity, ErrorSeverity::Info);
    }

    #[test]
    fn old_build_is_an_error() {
        let check = interpret(info(Some(18363)));
        assert_eq!(check.severity, ErrorSeverity::Error);
    }

    #[test]
    fn unreadable_build_is_a_warning_not_a_hard_failure() {
        let check = interpret(info(None));
        assert_eq!(check.severity, ErrorSeverity::Warning);
    }
}
