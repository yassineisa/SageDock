//! Shared types for system diagnostics. Kept separate from the individual checks so
//! the rollup logic (`compute_overall`) can be unit-tested against synthetic check
//! lists without touching the registry, WMI, or any other platform API.

use serde::Serialize;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::error::ErrorSeverity;

/// Outcomes produced by the diagnostic rollup; setup and recovery own their own states.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Healthy,
    DegradedButUsable,
    RestartRequired,
    Unsupported,
}

/// One diagnostic finding. A flat list of these (rather than a rigid struct with one
/// field per check) is what the frontend renders, new checks in later milestones are
/// additive without changing the wire shape.
#[derive(Debug, Clone, Serialize)]
pub struct CheckItem {
    /// Stable id, e.g. "windows_version", used by rollup logic and tests, never shown to the user.
    pub id: &'static str,
    pub label: String,
    pub severity: ErrorSeverity,
    /// Plain-language summary, always shown.
    pub summary: String,
    /// Technical detail for an optional "Show details" disclosure, raw registry values,
    /// command output, etc. Never the basis for a pass/fail decision, only for troubleshooting.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemCheckResult {
    pub overall: HealthState,
    pub checks: Vec<CheckItem>,
    pub checked_at: String,
}

impl SystemCheckResult {
    pub fn new(checks: Vec<CheckItem>) -> Self {
        Self {
            overall: compute_overall(&checks),
            checks,
            checked_at: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_default(),
        }
    }
}

/// Rolls per-check severities up into one overall state. Two checks get special-cased
/// by id because their meaning is more specific than a generic severity: an unsupported
/// Windows version blocks everything, and a pending reboot should be resolved before
/// anything else is attempted, but neither is simply "worse" than a plain error, the
/// user's next step is different (update Windows vs. restart vs. something SageDock can
/// retry). Everything else falls back to severity: any `Error`/`Fatal` makes the shell
/// usable but unable to proceed with setup, otherwise the machine looks ready.
///
/// Escalating on `reboot_pending` is specific rather than a catch-all, because
/// `system::reboot` only raises that check above `Info` when a restart genuinely blocks
/// setup. An advisory pending Windows update, or a file rename some other installer
/// scheduled, arrives here as `Info` and leaves the overall state alone. Before that
/// distinction existed, any one of three registry flags, one of which is set routinely by
/// ordinary software updates, forced the whole machine to `RestartRequired`.
fn compute_overall(checks: &[CheckItem]) -> HealthState {
    let find = |id: &str| checks.iter().find(|c| c.id == id);

    if let Some(check) = find("windows_version") {
        if check.severity == ErrorSeverity::Error || check.severity == ErrorSeverity::Fatal {
            return HealthState::Unsupported;
        }
    }

    if let Some(check) = find("reboot_pending") {
        if check.severity != ErrorSeverity::Info {
            return HealthState::RestartRequired;
        }
    }

    let has_blocking = checks
        .iter()
        .any(|c| matches!(c.severity, ErrorSeverity::Error | ErrorSeverity::Fatal));

    if has_blocking {
        HealthState::DegradedButUsable
    } else {
        HealthState::Healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &'static str) -> CheckItem {
        CheckItem {
            id,
            label: id.to_string(),
            severity: ErrorSeverity::Info,
            summary: String::new(),
            detail: None,
        }
    }

    fn with_severity(id: &'static str, severity: ErrorSeverity) -> CheckItem {
        CheckItem {
            severity,
            ..info(id)
        }
    }

    #[test]
    fn all_info_is_healthy() {
        let checks = vec![
            info("windows_version"),
            info("disk_space"),
            info("wsl_available"),
        ];
        assert_eq!(compute_overall(&checks), HealthState::Healthy);
    }

    #[test]
    fn unsupported_windows_version_wins_over_everything_else() {
        let checks = vec![
            with_severity("windows_version", ErrorSeverity::Error),
            info("reboot_pending"),
        ];
        assert_eq!(compute_overall(&checks), HealthState::Unsupported);
    }

    #[test]
    fn reboot_pending_overrides_generic_errors() {
        let checks = vec![
            info("windows_version"),
            with_severity("disk_space", ErrorSeverity::Error),
            with_severity("reboot_pending", ErrorSeverity::Warning),
        ];
        assert_eq!(compute_overall(&checks), HealthState::RestartRequired);
    }

    #[test]
    fn a_blocking_check_degrades_the_overall_state() {
        let checks = vec![
            info("windows_version"),
            with_severity("virtualization", ErrorSeverity::Error),
        ];
        assert_eq!(compute_overall(&checks), HealthState::DegradedButUsable);
    }

    /// Regression guard for the persistent restart warning. An advisory pending reboot is
    /// reported as `Info` by `system::reboot`, and must not escalate the whole machine to
    /// `RestartRequired` the way any non-`Info` severity used to.
    #[test]
    fn an_advisory_pending_reboot_leaves_the_overall_state_healthy() {
        let checks = vec![info("windows_version"), info("reboot_pending")];
        assert_eq!(compute_overall(&checks), HealthState::Healthy);
    }

    #[test]
    fn warnings_alone_stay_healthy() {
        let checks = vec![
            info("windows_version"),
            with_severity("disk_space", ErrorSeverity::Warning),
        ];
        assert_eq!(compute_overall(&checks), HealthState::Healthy);
    }
}
