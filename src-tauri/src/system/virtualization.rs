//! Hardware virtualization check.
//!
//! Two independent signals are read, and the order they're combined in matters:
//!
//! 1. `Win32_ComputerSystem.HypervisorPresent`, a hypervisor is currently running.
//! 2. `Win32_Processor.VirtualizationFirmwareEnabled`, firmware exposes VT-x/AMD-V.
//!
//! Signal 1 must be checked first, because once a hypervisor (Hyper-V, and therefore
//! WSL2 itself) is running, Windows is a guest on top of it and can no longer see the
//! raw CPU virtualization extensions, so signal 2 starts reporting `False` on machines
//! where virtualization demonstrably works. Reading only signal 2 produces a confident
//! "virtualization is turned off" on a perfectly healthy PC, sending the user into their
//! BIOS to fix a problem that doesn't exist. `HypervisorPresent: True` is proof that
//! virtualization works, since a hypervisor could not otherwise be running.
//!
//! Only when no hypervisor is running is the firmware flag meaningful, and only then can
//! this report virtualization as genuinely disabled.

use crate::error::ErrorSeverity;
use crate::system::process::run_hidden;
use crate::system::types::CheckItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualizationSignals {
    /// A hypervisor is currently running (conclusive proof virtualization works).
    pub hypervisor_present: Option<bool>,
    /// Firmware reports VT-x/AMD-V available. Unreliable while a hypervisor is running.
    pub firmware_enabled: Option<bool>,
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().trim_matches('"').to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn gather() -> VirtualizationSignals {
    // Both properties come back from one PowerShell launch (process startup dominates the
    // cost here), as two lines parsed positionally rather than by any localized label.
    let result = run_hidden(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$h=(Get-CimInstance Win32_ComputerSystem).HypervisorPresent; \
             $v=(Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty VirtualizationFirmwareEnabled); \
             Write-Output ([string]$h); Write-Output ([string]$v)",
        ],
    );

    let Ok(Some(result)) = result else {
        return VirtualizationSignals {
            hypervisor_present: None,
            firmware_enabled: None,
        };
    };

    if !result.success {
        return VirtualizationSignals {
            hypervisor_present: None,
            firmware_enabled: None,
        };
    }

    let mut lines = result.combined_output.lines();
    VirtualizationSignals {
        hypervisor_present: lines.next().and_then(parse_bool),
        firmware_enabled: lines.next().and_then(parse_bool),
    }
}

fn interpret(signals: VirtualizationSignals) -> CheckItem {
    let detail = Some(format!(
        "HypervisorPresent={:?} | VirtualizationFirmwareEnabled={:?}",
        signals.hypervisor_present, signals.firmware_enabled
    ));

    let item = |severity, summary: &str| CheckItem {
        id: "virtualization",
        label: "Virtualization".into(),
        severity,
        summary: summary.into(),
        detail: detail.clone(),
    };

    // A running hypervisor settles the question on its own.
    if signals.hypervisor_present == Some(true) {
        return item(ErrorSeverity::Info, "Virtualization is turned on.");
    }

    match signals.firmware_enabled {
        Some(true) => item(ErrorSeverity::Info, "Virtualization is turned on."),
        Some(false) => item(
            ErrorSeverity::Error,
            "Virtualization is turned off. Your computer supports the technology SageDock needs, but it's currently switched off in your computer's firmware settings.",
        ),
        None => item(
            ErrorSeverity::Warning,
            "SageDock couldn't automatically confirm whether virtualization is turned on. This will be checked again during setup.",
        ),
    }
}

pub fn check() -> CheckItem {
    interpret(gather())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals(hypervisor: Option<bool>, firmware: Option<bool>) -> VirtualizationSignals {
        VirtualizationSignals {
            hypervisor_present: hypervisor,
            firmware_enabled: firmware,
        }
    }

    #[test]
    fn firmware_enabled_is_healthy() {
        assert_eq!(
            interpret(signals(Some(false), Some(true))).severity,
            ErrorSeverity::Info
        );
    }

    /// Regression test for a real false positive: on a machine with Hyper-V running, the
    /// CPU reports `VirtualizationFirmwareEnabled=False` even though virtualization
    /// obviously works. Reading the firmware flag alone told a healthy PC to visit its BIOS.
    #[test]
    fn running_hypervisor_overrides_a_false_firmware_flag() {
        let check = interpret(signals(Some(true), Some(false)));
        assert_eq!(check.severity, ErrorSeverity::Info);
    }

    #[test]
    fn genuinely_disabled_is_an_error() {
        assert_eq!(
            interpret(signals(Some(false), Some(false))).severity,
            ErrorSeverity::Error
        );
    }

    #[test]
    fn undetectable_is_a_soft_warning_not_a_failure() {
        assert_eq!(
            interpret(signals(None, None)).severity,
            ErrorSeverity::Warning
        );
    }

    #[test]
    fn hypervisor_present_with_unknown_firmware_is_healthy() {
        assert_eq!(
            interpret(signals(Some(true), None)).severity,
            ErrorSeverity::Info
        );
    }

    #[test]
    fn parses_powershell_boolean_strings() {
        assert_eq!(parse_bool("True"), Some(true));
        assert_eq!(parse_bool("False"), Some(false));
        assert_eq!(parse_bool(""), None);
    }
}
