# The restart warning: a worked example

Written for a reviewer, as a case study. This is the one piece of logic in SageDock with a
documented bug history, two rounds of correction, and a subtle failure mode. It is also
small enough to read completely, which makes it the best place to see the codebase's
conventions applied to a real problem.

If you are auditing one thing, audit this.

## The symptom

SageDock reported "Windows needs to restart" on essentially every machine, every run,
including machines that had just restarted.

## Root cause

Windows exposes several unrelated "a restart is pending" indicators. The original
`system/reboot.rs` OR-ed three of them and returned `ErrorSeverity::Warning` if any was
set:

| Indicator                                         | Meaning                                       | Set by            |
| ------------------------------------------------- | --------------------------------------------- | ----------------- |
| `...\Component Based Servicing\RebootPending`     | Windows servicing needs a restart             | Windows           |
| `...\WindowsUpdate\Auto Update\RebootRequired`    | Windows Update needs a restart                | Windows           |
| `...\Session Manager\PendingFileRenameOperations` | _someone_ scheduled a file swap for next boot | **any installer** |

The third is the culprit. Any program calling
`MoveFileEx(..., MOVEFILE_DELAY_UNTIL_REBOOT)` sets it, and it persists until the next
boot. Browser updaters, antivirus definition updates and OEM utilities set it constantly.

**Evidence gathered on the development machine**, by read-only registry inspection:

- `PendingFileRenameOperations`: **present, 26 entries**, every one a Lenovo updater DLL
  under `C:\ProgramData\lenovo\UDC\Hosts\x64\...`.
- `Component Based Servicing\RebootPending`: **absent**.
- `WindowsUpdate\Auto Update\RebootRequired`: **absent**.

So no Windows update was pending at all, and the message shown to the user was simply
false. The read genuinely succeeded rather than failing silently: the value is
`REG_MULTI_SZ`, and `winreg` 0.55's `FromRegValue for String` accepts
`REG_SZ | REG_EXPAND_SZ | REG_MULTI_SZ`, joining multi-string entries with newlines.

Two things then amplified it:

1. `compute_overall` in `system/types.rs` escalated **any** non-`Info` severity on the
   `reboot_pending` check to `HealthState::RestartRequired`, so one line of vendor noise
   downgraded the entire machine's health.
2. Worse, `runtime/provision.rs` consulted the same predicate after installing Windows
   components, so third-party file-rename noise could divert a **clean, working install**
   into the `AwaitingRestart` state.

## The corrected logic

Read [`src-tauri/src/system/reboot.rs`](../../src-tauri/src/system/reboot.rs). The three
indicators are now kept apart in a `RebootIndicators` struct, and the decision is a pure
function of those indicators plus a `RestartContext` supplied by the caller:

```rust
pub struct RestartContext {
    pub wsl_available: bool,          // wsl.exe answers
    pub vm_platform_present: bool,    // the Host Compute Service exists
    pub setup_awaiting_restart: bool, // setup recorded a restart for itself
}

fn restart_blocks_setup(context: RestartContext) -> bool {
    context.setup_awaiting_restart
}
```

The resulting severities:

| Condition                                     | Severity  | Message intent                                              |
| --------------------------------------------- | --------- | ----------------------------------------------------------- |
| Setup recorded a restart request              | `Warning` | A genuine blocker. Restart, reopen, continue setup.         |
| `wsl.exe` answers but no Host Compute Service | `Info`    | Installation advice: continue setup to install or check it. |
| CBS or Windows Update pending                 | `Info`    | Real, worth doing, but does not stop SageDock.              |
| Only `PendingFileRenameOperations`            | `Info`    | Another program's business; does not affect SageDock.       |
| Nothing                                       | `Info`    | "No restart is needed."                                     |

`servicing_pending()` deliberately excludes `file_rename_scheduled`. The comment states the
reason directly: a third-party updater scheduling a DLL swap must never be able to divert
SageDock's setup into a restart prompt.

Note what was **not** done: nothing is suppressed. A pending Windows update is still
reported, still visible in Diagnostics, just not as a blocker. No Windows Update state is
modified and no registry value is deleted.

## The second correction, from a follow-up audit

The first fix left two defects, both found by a later audit recorded in
[HOME-LAUNCH-AUDIT.md](../../coding%20agent%20onboarding/HOME-LAUNCH-AUDIT.md):

**1. A missing service does not prove a feature is awaiting a reboot.** The first fix
treated "`wsl --status` succeeds but `sc query vmcompute` fails" as a genuine
enable-pending-reboot state. But that same signal also describes a machine where the
feature was simply never installed. It now produces installation advice (`Info`), which is
why `restart_blocks_setup` reduces to `setup_awaiting_restart` alone.

**2. A saved flag survives a reboot forever.** `PersistedSetupState.awaiting_restart` is
written to disk. Once set, it stayed set, so restarting the computer did not clear the
warning, the exact symptom the user reported, reintroduced by the fix's own state.

The remedy is in
[`runtime/provision.rs`](../../src-tauri/src/runtime/provision.rs):

```rust
pub struct PersistedSetupState {
    pub awaiting_restart: bool,
    #[serde(default)]
    pub restart_requested_at: Option<u64>,   // added
    pub runtime_version: Option<String>,
    pub completed: bool,
}
```

`awaiting_restart_now(dir)` compares `restart_requested_at` with Windows' own boot
timestamp, and returns `false` once a later boot is confirmed. Three details matter:

- **Legacy state files** predating the timestamp fall back to the state file's modification
  time.
- **Unknown boot information retains the request.** If the CIM query fails, the code does
  _not_ claim a reboot happened. It fails toward showing the warning, while "Continue
  setup" still validates the real components, so an unavailable CIM service cannot trap the
  user in a restart loop.
- **`completed` short-circuits everything**, so a finished setup cannot hold a stale
  warning.

The boot time comes from
[`Win32_OperatingSystem.LastBootUpTime`](https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-operatingsystem)
via `system::boot_started_at()`: a hidden PowerShell CIM query with a five-second timeout,
cached in a `OnceLock` after a successful read. A normal launch with no outstanding request
never makes the query.

## Where it is consumed

The parameter is threaded rather than read globally, so the decision stays testable:

- `system::run_system_check(setup_awaiting_restart: bool)` builds the `RestartContext`.
- [`runtime/health.rs`](../../src-tauri/src/runtime/health.rs) `preflight` passes
  `PersistedSetupState::load(data).awaiting_restart_now(data)`. Note its comment: preflight
  aborts only on `Error`/`Fatal`, so a restart advisory has never gated setup.
- [`desktop.rs`](../../src-tauri/src/desktop.rs) `diagnostic_report` does the same, and now
  also emits `overall` in the report JSON. Before this work `overall` was computed and
  discarded, meaning `HealthState::RestartRequired` had **no UI consumer at all**.

## Test coverage

Ten tests in `reboot.rs`, eight of them against the pure `interpret`:

- **False positives:** a third-party scheduled file rename is `Info`; a pending Windows
  update alone is `Info`; a machine with no WSL is not reported as needing a restart.
- **Genuine blockers:** a restart recorded by setup is `Warning`; a blocking restart
  outranks advisory indicators; a missing platform without a setup request is _not_ a
  warning.
- **Clearing:** the warning becomes `Info` once the platform is present; a clean machine
  says "No restart is needed."
- **Predicate:** `servicing_pending()` ignores file renames and honours CBS/Windows Update.

Plus `an_advisory_pending_reboot_leaves_the_overall_state_healthy` in `system/types.rs`
guarding the rollup, and tests in `provision.rs` covering restart requests across boot
changes.

## Residual risk

- The comparison uses UTC timestamps, so changing the system clock, or altering a legacy
  state file's modification time, can affect the inference.
- **No test has observed a real reboot transition.** The decision logic is unit-tested; the
  actual Windows feature-enable → reboot → resume path needs a clean-Windows VM.
- The root cause is proven on one machine. A different machine showing the warning could
  legitimately have a CBS or Windows Update indicator instead, which the new logic reports
  as `Info`. That is the intended behaviour, but it means the same symptom can have more
  than one source.
