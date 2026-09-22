//! Pending-restart detection.
//!
//! Windows exposes several unrelated "a restart is pending" indicators, and they do not
//! mean the same thing. Treating them as one boolean is what made SageDock report that a
//! restart was needed on essentially every machine:
//!
//! - `Component Based Servicing\RebootPending` and `WindowsUpdate\Auto Update\RebootRequired`
//!   are set by Windows servicing. They are real, but a pending Windows update does not stop
//!   SageDock installing or running.
//! - `PendingFileRenameOperations` is set by *any* installer that schedules a file
//!   replacement for next boot via `MoveFileEx(..., MOVEFILE_DELAY_UNTIL_REBOOT)`. Browser
//!   updaters, antivirus definition updates, and OEM utilities set it constantly, and it
//!   persists until the next reboot. On the development machine it held 26 entries, all
//!   belonging to a Lenovo updater, while both servicing indicators were absent. It says
//!   nothing whatsoever about whether SageMath can run.
//!
//! Setup records restart requests after Windows component installation or a specific
//! runtime failure. A missing service alone does not prove the feature was enabled: it
//! may simply never have been installed. Only an outstanding setup request is a warning.
//! The caller excludes requests made before the current Windows boot.
//!
//! None of these reads require elevation, and nothing here modifies Windows Update state
//! or removes a registry value.

use winreg::enums::*;
use winreg::{RegKey, HKEY};

use crate::error::ErrorSeverity;
use crate::system::types::CheckItem;

fn key_exists(hive: HKEY, path: &str) -> bool {
    RegKey::predef(hive).open_subkey(path).is_ok()
}

/// Reads a value only to learn whether it is present.
///
/// `PendingFileRenameOperations` is `REG_MULTI_SZ`, which winreg's `String` conversion
/// accepts (joining the entries with newlines), so presence is detected correctly. The
/// contents are deliberately not inspected: which files some other installer intends to
/// replace is none of SageDock's business and is not evidence about SageMath.
fn value_exists(hive: HKEY, path: &str, value: &str) -> bool {
    RegKey::predef(hive)
        .open_subkey(path)
        .and_then(|key| key.get_value::<String, _>(value))
        .is_ok()
}

/// The raw indicators, kept apart because they carry different weight.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RebootIndicators {
    /// Windows component servicing is waiting on a restart.
    pub component_servicing: bool,
    /// Windows Update is waiting on a restart.
    pub windows_update: bool,
    /// Some installer scheduled file replacements for the next boot. Very weak: this is
    /// true on a large share of perfectly healthy machines.
    pub file_rename_scheduled: bool,
}

impl RebootIndicators {
    /// Whether Windows' own *servicing* stack is waiting on a restart.
    ///
    /// Excludes `PendingFileRenameOperations` on purpose. A third-party updater scheduling
    /// a DLL swap must never be able to divert SageDock's setup into a restart prompt.
    pub fn servicing_pending(self) -> bool {
        self.component_servicing || self.windows_update
    }
}

pub(crate) fn gather() -> RebootIndicators {
    RebootIndicators {
        component_servicing: key_exists(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending",
        ),
        windows_update: key_exists(
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired",
        ),
        file_rename_scheduled: value_exists(
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager",
            "PendingFileRenameOperations",
        ),
    }
}

/// Everything the decision needs, gathered by the caller so `interpret` stays pure and
/// testable without a registry, a WSL installation, or a specific machine.
#[derive(Debug, Clone, Copy)]
pub struct RestartContext {
    /// `wsl.exe` responds.
    pub wsl_available: bool,
    /// The Host Compute Service exists. This does not by itself prove WSL 2 can run.
    pub vm_platform_present: bool,
    /// Setup has recorded that it is waiting for a restart of its own.
    pub setup_awaiting_restart: bool,
}

/// A missing feature calls for setup; only setup can establish a restart requirement.
fn restart_blocks_setup(context: RestartContext) -> bool {
    context.setup_awaiting_restart
}

fn interpret(indicators: RebootIndicators, context: RestartContext) -> CheckItem {
    let item = |severity: ErrorSeverity, summary: &str, detail: Option<String>| CheckItem {
        id: "reboot_pending",
        label: "Restart status".into(),
        severity,
        summary: summary.into(),
        detail,
    };

    // A genuine blocker outranks whatever the registry happens to say.
    if restart_blocks_setup(context) {
        return item(
            ErrorSeverity::Warning,
            "Windows needs to restart to finish switching on the feature SageDock uses to run SageMath. Restart, then reopen SageDock and continue setup.",
            Some(format!(
                "blocking: setup_awaiting_restart={} wsl_available={} vm_platform_present={}",
                context.setup_awaiting_restart, context.wsl_available, context.vm_platform_present
            )),
        );
    }

    if context.wsl_available && !context.vm_platform_present {
        return item(ErrorSeverity::Info,
            "A Windows component SageDock needs is not available. Continue setup to install or check it.", None);
    }

    // Past here nothing is blocking SageDock, so nothing is a warning. The indicators are
    // still reported, because silently dropping them would hide a real pending update.
    if indicators.servicing_pending() {
        return item(
            ErrorSeverity::Info,
            "Windows has an update waiting for a restart. That is worth doing when convenient, but it does not stop SageDock working.",
            Some(format!(
                "advisory: component_servicing={} windows_update={}",
                indicators.component_servicing, indicators.windows_update
            )),
        );
    }

    if indicators.file_rename_scheduled {
        return item(
            ErrorSeverity::Info,
            "Another program has scheduled file updates for your next restart. This does not affect SageDock.",
            Some(
                "advisory: PendingFileRenameOperations is present. Set by any installer that \
                 schedules a file replacement for next boot; not a Windows servicing signal."
                    .into(),
            ),
        );
    }

    item(ErrorSeverity::Info, "No restart is needed.", None)
}

pub fn check(context: RestartContext) -> CheckItem {
    interpret(gather(), context)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A healthy machine: WSL present and working, nothing pending.
    const HEALTHY: RestartContext = RestartContext {
        wsl_available: true,
        vm_platform_present: true,
        setup_awaiting_restart: false,
    };

    /// A machine that has never had SageDock set up. Not blocked; setup installs WSL.
    const FRESH: RestartContext = RestartContext {
        wsl_available: false,
        vm_platform_present: false,
        setup_awaiting_restart: false,
    };

    fn only_file_renames() -> RebootIndicators {
        RebootIndicators {
            file_rename_scheduled: true,
            ..Default::default()
        }
    }

    // --- false positives -------------------------------------------------------------

    /// The reported bug, reproduced. `PendingFileRenameOperations` was present on the
    /// development machine (26 entries from a Lenovo updater) while both Windows servicing
    /// indicators were absent, and it made every diagnostic run show a restart warning.
    #[test]
    fn a_third_party_scheduled_file_rename_is_not_a_warning() {
        let check = interpret(only_file_renames(), HEALTHY);
        assert_eq!(check.severity, ErrorSeverity::Info);
        assert!(
            check.summary.contains("does not affect SageDock"),
            "summary should say it is harmless: {}",
            check.summary
        );
    }

    /// A pending Windows update is real and worth mentioning, but it does not stop
    /// SageDock, so it must not be presented as a blocker either.
    #[test]
    fn a_pending_windows_update_alone_is_informational() {
        let indicators = RebootIndicators {
            windows_update: true,
            component_servicing: true,
            file_rename_scheduled: true,
        };
        let check = interpret(indicators, HEALTHY);
        assert_eq!(check.severity, ErrorSeverity::Info);
        assert!(check.summary.contains("does not stop SageDock"));
    }

    /// A machine with no WSL yet is not "blocked by a restart", setup installs WSL. The
    /// old logic would warn here purely because some installer had queued a file rename.
    #[test]
    fn a_machine_without_wsl_is_not_reported_as_needing_a_restart() {
        assert_eq!(
            interpret(only_file_renames(), FRESH).severity,
            ErrorSeverity::Info
        );
    }

    // --- genuine, setup-blocking restarts --------------------------------------------

    /// WSL 1 can exist without the WSL 2 platform ever having been installed.
    #[test]
    fn a_missing_platform_without_a_setup_request_is_not_a_restart_warning() {
        let context = RestartContext {
            wsl_available: true,
            vm_platform_present: false,
            setup_awaiting_restart: false,
        };
        let check = interpret(RebootIndicators::default(), context);
        assert_eq!(check.severity, ErrorSeverity::Info);
        assert!(check.summary.contains("Continue setup"));
    }

    /// A restart setup recorded for itself must survive, whatever the registry says.
    #[test]
    fn a_restart_recorded_by_setup_is_a_warning() {
        let context = RestartContext {
            wsl_available: false,
            vm_platform_present: false,
            setup_awaiting_restart: true,
        };
        assert_eq!(
            interpret(RebootIndicators::default(), context).severity,
            ErrorSeverity::Warning
        );
    }

    /// A blocking restart outranks the noisy indicators rather than being masked by them.
    #[test]
    fn a_blocking_restart_wins_over_advisory_indicators() {
        let context = RestartContext {
            wsl_available: true,
            vm_platform_present: false,
            setup_awaiting_restart: true,
        };
        let check = interpret(
            RebootIndicators {
                component_servicing: true,
                windows_update: true,
                file_rename_scheduled: true,
            },
            context,
        );
        assert_eq!(check.severity, ErrorSeverity::Warning);
    }

    // --- clearing --------------------------------------------------------------------

    /// Once the blocking condition resolves, the warning goes away on the next check.
    /// Detection reads live state every time and persists nothing of its own.
    #[test]
    fn the_warning_clears_once_the_request_has_been_satisfied() {
        let blocked = RestartContext {
            wsl_available: true,
            vm_platform_present: false,
            setup_awaiting_restart: true,
        };
        assert_eq!(
            interpret(RebootIndicators::default(), blocked).severity,
            ErrorSeverity::Warning
        );

        // Same indicators, platform now active: back to Info.
        assert_eq!(
            interpret(RebootIndicators::default(), HEALTHY).severity,
            ErrorSeverity::Info
        );
    }

    #[test]
    fn a_completely_clean_machine_says_no_restart_is_needed() {
        let check = interpret(RebootIndicators::default(), HEALTHY);
        assert_eq!(check.severity, ErrorSeverity::Info);
        assert_eq!(check.summary, "No restart is needed.");
        assert!(check.detail.is_none());
    }

    // --- the servicing predicate ------------------------------------------------------

    /// The weak indicator must never count as Windows servicing. This is the distinction
    /// the whole fix rests on.
    #[test]
    fn the_servicing_predicate_ignores_scheduled_file_renames() {
        assert!(!only_file_renames().servicing_pending());
        assert!(only_file_renames().file_rename_scheduled);
    }

    #[test]
    fn the_servicing_predicate_honours_windows_servicing() {
        let cbs = RebootIndicators {
            component_servicing: true,
            ..Default::default()
        };
        let wu = RebootIndicators {
            windows_update: true,
            ..Default::default()
        };
        assert!(cbs.servicing_pending());
        assert!(wu.servicing_pending());
        assert!(!RebootIndicators::default().servicing_pending());
    }
}
