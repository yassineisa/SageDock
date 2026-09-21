//! System diagnostics: read-only checks of Windows/WSL state, reported as structured
//! results instead of raw command output. Each check lives in its own module, split
//! into an untested "gather" step (the actual registry/WMI/process call) and a unit
//! tested "interpret" step (pure logic turning raw data into a `CheckItem`) — see the
//! testing philosophy note in each file.
//!
//! These checks are read-only. The runtime module owns installation and recovery.

mod architecture;
pub(crate) mod disk_space;
/// Shared with `crate::runtime`, which spawns WSL commands through the same helper so
/// console-window suppression and output decoding behave identically everywhere.
pub(crate) mod process;
mod reboot;
mod types;
mod virtualization;
mod windows_version;
mod wsl;

pub use types::SystemCheckResult;

/// Stable boot timestamp from Windows, cached only after a successful read. This is used
/// only while setup has a saved restart request, never on the normal launch path.
pub fn boot_started_at() -> Option<u64> {
    static BOOT: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    if let Some(value) = BOOT.get() {
        return Some(*value);
    }
    let result = process::run_hidden_timeout(
        "powershell.exe",
        &[
            "-NoProfile", "-NonInteractive", "-Command",
            "$ErrorActionPreference='Stop'; ([DateTimeOffset](Get-CimInstance Win32_OperatingSystem).LastBootUpTime).ToUnixTimeSeconds()",
        ],
        std::time::Duration::from_secs(5),
    ).ok()??;
    if !result.success {
        return None;
    }
    let value = result.combined_output.trim().parse::<u64>().ok()?;
    BOOT.set(value).ok();
    Some(value)
}

/// Runs every check and rolls them up into one result.
///
/// `setup_awaiting_restart` is the restart setup has recorded for itself, read from
/// persisted setup state by the caller. It is passed in rather than read here because this
/// module is deliberately free of app-data paths, and because a pure input keeps the
/// restart decision unit-testable.
///
/// Checks that shell out (virtualization, WSL availability, the virtual machine platform)
/// make this call take on the order of a few hundred milliseconds to a couple of seconds —
/// fine for an explicit "Run diagnostic" action, but not something to run unprompted on
/// every app launch.
pub fn run_system_check(setup_awaiting_restart: bool) -> SystemCheckResult {
    // Observations provide installation advice, not proof that a reboot is required.
    // Only the outstanding request from setup can justify a blocking restart warning.
    let restart_context = reboot::RestartContext {
        wsl_available: crate::runtime::wsl::is_available(),
        vm_platform_present: crate::runtime::wsl::vm_platform_present(),
        setup_awaiting_restart,
    };

    let checks = vec![
        windows_version::check(),
        architecture::check(),
        virtualization::check(),
        disk_space::check(),
        wsl::check_available(),
        wsl::check_default_version(),
        wsl::check_sagedock_distro(),
        reboot::check(restart_context),
    ];

    SystemCheckResult::new(checks)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the real registry/WMI/`wsl.exe` calls against whatever machine `cargo
    /// test` runs on, rather than the mocked `interpret_*` unit tests elsewhere in this
    /// module tree. Ignored by default since its result is environment-dependent (and,
    /// unlike the rest of the suite, genuinely touches the OS) — run explicitly with:
    ///   cargo test -p sagedock system::tests::smoke_check_runs_against_this_machine -- --ignored --nocapture
    #[test]
    #[ignore]
    fn smoke_check_runs_against_this_machine() {
        let result = run_system_check(false);
        assert_eq!(
            result.checks.len(),
            8,
            "expected one CheckItem per registered check"
        );
        for check in &result.checks {
            assert!(
                !check.summary.is_empty(),
                "{} produced an empty summary",
                check.id
            );
        }
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    }
}
