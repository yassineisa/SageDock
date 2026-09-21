# Setup reliability — implementation handoff

Audience: the engineer auditing this change, and whoever maintains setup next. Assumes
familiarity with the repository layout in `coding agent onboarding/REPO_CONTEXT.md`.

This change makes the setup operation belong to the backend, gives it an authoritative
queryable state, and replaces the elevation call with a supervised one. It does not claim
setup is now reliable on hardware it has never run on — see
[§7 What remains untested](#7-what-remains-untested-and-how-to-test-it), which is the
section an auditor should read first.

---

## 1. Confirmed root causes

Each of these was read out of the code at a specific line, not inferred from the symptom.
Where I could not confirm something, it is in [§1.7 Hypotheses](#17-hypotheses-not-confirmed)
instead.

### 1.1 Setup state was owned by the screen that started it — confirmed

`Home.tsx` held the only subscription to `setup-progress`, in an effect with `[]`
dependencies (old line 112). React tears that down on unmount, so **every progress event
emitted while the user was on another page was delivered to nobody and then discarded** —
there was no snapshot to recover from, because the backend published events and kept no
state.

Three separate consequences, all reported as one bug:

- The progress card rendered only when `progress && task.busy` (old line 632). After
  navigating away and back, `progress` was `null`, so **no progress was shown at all**.
- `refresh()` was gated on `if (!task.busy)` (old line 108). While setup ran, Home refused
  to re-read its own status, so a remounted Home was stuck on `{!status && …}` →
  **"Checking SageMath…" indefinitely**, with every button disabled.
- `task.busy` was a frontend string cleared in the `finally` of the `run_setup` promise
  (`TaskContext.tsx:57`), rendered app-wide by `AppShell.tsx:40`. **If that promise never
  settled, "Setting up SageMath" was permanent and there was no way back.**

That last point is the literal mechanism behind reported problem 3.

### 1.2 The elevated install emitted nothing for up to 30 minutes — confirmed

`provision.rs` emitted one progress event before elevation ("Windows will ask for
permission to continue"), then called `wsl::install_wsl_elevated()`, which blocked in
`run_hidden_timeout(..., Duration::from_secs(1800))`.

Between those two points **no event of any kind was emitted**. The elevated script also
discarded all output (`dism … *> $null`), so there was nothing to stream even in principle.
For up to thirty minutes the UI's newest information was a sentence about a prompt that
may already have been answered.

### 1.3 Three different situations were indistinguishable — confirmed

"SageDock is working", "Windows is applying changes", and "a permission prompt nobody has
noticed" all rendered as the same indefinite spinner with the same text. This is why an
unanswered UAC prompt _looked_ like a freeze: there was no state in the system that could
have said otherwise.

Compounding it: the outer PowerShell was launched with `CREATE_NO_WINDOW`
(`process.rs:45`) and `-WindowStyle Hidden`. A consent dialog owned by a hidden process can
open behind the app window or only flash in the taskbar, so the prompt genuinely can go
unnoticed.

### 1.4 The elevation result channel was lossy, and could report a false success — confirmed by reading

The only thing returned from the elevated helper was one integer, and the launcher's
`catch` collapsed every distinct failure into `exit 1`:

```powershell
}} catch {{ if ($_.Exception.NativeErrorCode -eq 1223) {{ exit 1223 }}; exit 1 }}
```

So "the helper could not start", "policy blocked elevation", and "dism failed with code 1"
were the same answer, despite needing different advice.

Worse, the success path was `exit $p.ExitCode` from `Start-Process -PassThru -Wait`.
`ExitCode` on that process object is **null** in several documented situations; `exit $null`
in PowerShell exits **0**, which `classify_install_exit` mapped to `Completed`. A run that
never happened could therefore be reported as a successful install, after which setup would
continue and fail later at `--import` with a confusing error.

I confirmed this by reading the code path, not by reproducing it. It is the strongest
correctness argument for the new result channel, and I have flagged it as read-confirmed
rather than observed.

### 1.5 The timeout killed the wrong process — confirmed

`run_hidden_timeout` on expiry does `child.kill()` — the **outer, unelevated** PowerShell.
It cannot kill the elevated `dism` it started, and `kill()` does not touch grandchildren
regardless. The installation continued invisibly while the app reported that the operation
"exceeded its time limit", directly violating the standing rule that a timeout must not
falsely report a stopped installation.

### 1.6 "What happens during setup?" linked to the wrong page — confirmed

`Home.tsx:658` was `<Link to="/help">`, the general questions page. Besides answering a
different question, **navigating there was itself the surest way to trigger §1.1** — the
control most likely to be clicked by a confused user during setup was the one that
destroyed the progress view.

### 1.7 Hypotheses (not confirmed)

- **UAC prompt z-order.** That a consent dialog owned by a hidden process can appear behind
  the app is well documented and consistent with the report, but I did not reproduce it on
  this machine (which already has WSL, so elevation never runs). The change addresses it by
  _telling the user where to look_, which is correct whether or not z-order is the cause.
- **Which of §1.1's three consequences the user actually hit.** All three are real and all
  three are fixed; I cannot say from the report which produced the "indefinitely" they saw.

---

## 2. Files changed, and why

### New

| File                                | Why                                                                                                                                                     |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src-tauri/src/setup.rs`            | The backend-owned operation: `SetupTracker`, `SetupSnapshot`, phases, the step plan, persistence and crash reconciliation, and log redaction. 18 tests. |
| `src/state/SetupContext.tsx`        | One app-wide observer, mounted above the router. Holds the sequence rule that makes reconnection safe.                                                  |
| `src/components/SetupProgress.tsx`  | The live view: who we are waiting on, elapsed time, last real progress, step list, technical log, diagnostic export.                                    |
| `src/components/SetupExplainer.tsx` | The dedicated explanation that replaces the `/help` link.                                                                                               |

### Changed

| File                                 | Change                                                                                                                                                                                                                                                                                                                    |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src-tauri/src/runtime/wsl.rs`       | `install_wsl_elevated` rewritten: no `-Wait`, returns the helper pid, supervises with real liveness, reads a JSON result keyed to the operation. Adds `ElevationEvent`, `supervise_helper`, `read_report`, `classify_report`, `parse_helper_pid`, `elevated_script`, `ps_single_quoted`. Removes `classify_install_exit`. |
| `src-tauri/src/runtime/provision.rs` | `run_setup` takes an `operation_id`; `SetupProgress` gains `ProgressKind` so waiting and heartbeats are distinguishable from progress.                                                                                                                                                                                    |
| `src-tauri/src/commands.rs`          | `run_setup` now spawns a worker and returns the opening snapshot immediately. Adds `setup_snapshot`, `acknowledge_setup_interruption`, `setup_diagnostics`, `EventSink`, `ThreadOperation`.                                                                                                                               |
| `src-tauri/src/state.rs`             | Adds the `setup` tracker, plus `claim_operation`/`end_operation` for a lock that outlives a command.                                                                                                                                                                                                                      |
| `src-tauri/src/lib.rs`               | Registers the new commands; reconciles a previous run's record at startup.                                                                                                                                                                                                                                                |
| `src/pages/Home.tsx`                 | Stops owning setup. Renders the shared progress view, opens the explainer in place, and reads busy-ness from the snapshot.                                                                                                                                                                                                |
| `src/components/AppShell.tsx`        | App-wide setup strip with a route back to it.                                                                                                                                                                                                                                                                             |
| `src/lib/commands.ts`                | Snapshot types and the new command bindings.                                                                                                                                                                                                                                                                              |
| `src/styles/layout.css`              | Styles built from the existing burgundy/status tokens; no new colours. Also caps `.dialog` height and scrolls `.dialog-body` — a general fix, see §6.                                                                                                                                                                     |
| `tests/ui/workspace.spec.ts`         | Fixture now models the backend (snapshot + sequence + event emission, surviving reload); 13 new tests.                                                                                                                                                                                                                    |

---

## 3. Setup states and permitted transitions

`SetupPhase` in `setup.rs`. Deliberately coarser than `SetupStage`: a **stage** names the
step, a **phase** says whether anybody needs to act.

```text
                    ┌──────────────► Completed        (verified end to end)
                    │
Idle ──► Running ───┼──────────────► RestartRequired  (Windows said so, via its own exit code)
  ▲        │  ▲     │
  │        │  │     └──────────────► Failed           (carries an AppError with a next step)
  │        ▼  │
  │   WaitingForPermission          Interrupted       (reconciled at startup, never entered live)
  │        │  ▲                           │
  │        ▼  │                           │
  │   WaitingForWindows                   │
  │                                       │
  └───────────────────────────────────────┘
        (acknowledge, or start a new run)
```

Rules the code enforces:

- **`Running` ⇄ `WaitingForPermission` ⇄ `WaitingForWindows`** are freely reversible while
  the operation is live. Returning to `Running` is explicit (`commands.rs`, `ProgressKind::Working`
  restores the phase), so the UI cannot keep asking for a permission already granted.
- **Every terminal phase is persisted** before the worker exits.
- **`Interrupted` is only ever produced by `reconcile`**, never entered by a live run. It
  means "a previous process left a record saying it was running, and that process is gone".
- **`Idle` is reachable from `Interrupted`** only via
  `acknowledge_setup_interruption`, so an interruption is reported once rather than every
  launch.

### The sequence rule

Every published snapshot carries `operation_id` and a monotonic `seq`. Observers apply a
snapshot only if `operation_id` differs (a new run is always newer information, even though
its `seq` restarts at 0) or `seq` is strictly greater. This is what stops a slow
`setup_snapshot()` query, answering after a fast event, from rolling the display backwards.

`heartbeat()` bumps `seq` and `heartbeat_at` but deliberately **not** `updated_at`, so
"last real progress" keeps ageing during a silent stage. That is what the UI uses to decide
when to say a step is running long, and it is why a heartbeat can never masquerade as
progress.

---

## 4. Elevation, timeouts, interruption, retry

### Elevation

The launcher no longer waits. It calls `Start-Process -Verb RunAs -PassThru`, prints
`SAGEDOCK_PID=<id>`, and exits. From there:

| Situation              | How it is detected                 | What the user sees                                         |
| ---------------------- | ---------------------------------- | ---------------------------------------------------------- |
| Answering the prompt   | Launcher still running, no pid yet | "Waiting for you", with where to look for the prompt       |
| Declined               | Launcher exits `1223`              | A warning, not an error; setup can be started again        |
| Helper failed to start | Launcher exits without a pid       | Distinct error naming the launcher's own message           |
| Helper working         | pid alive, no result yet           | "Waiting for Windows", heartbeat every 500 ms              |
| Restart needed         | Result `restart_required`          | The restart flow                                           |
| Completed              | Result `completed`                 | Setup continues                                            |
| Helper vanished        | pid gone for >10 s with no result  | Error naming the pid; nothing is assumed to have succeeded |

The helper writes a JSON report in a `finally` block, so every path leaves a result — a
helper that dies without one is _detectable_ (the supervisor sees the process disappear)
even though it cannot be diagnosed.

**Result channel scope.** The file is `elevated-<operation_id>.json` in SageDock's own
application data directory, and a report is accepted only if its embedded `operation_id`
matches the current run. That defends against a stale or concurrent run's result being read
as this one's. It is **not** a defence against an attacker who can already write to the
user's application data — such an attacker has easier targets there, and this is documented
in the function's doc comment rather than overstated. The code the helper runs is still
passed immutably as an `-EncodedCommand`, so no executable script is written to disk.

Both values interpolated into the script go through `ps_single_quoted` (doubling embedded
quotes). Both are SageDock-generated rather than user-supplied, so this is defence in depth
rather than the primary control, and there is a test for the injection shape.

### Timeouts

- **Consent: 600 s.** Bounds "nobody answered the prompt". Reported as a warning — nothing
  was installed and nothing was left running.
- **Helper: none.** Deliberate. The helper is supervised by liveness, not by a clock,
  because `dism` is routinely silent for minutes and a clock cannot tell that from a hang.
- **Result grace: 10 s.** Only covers the gap between the process exiting and its file
  becoming readable.

**Nothing is ever killed.** An unelevated process cannot stop an elevated one, and the old
code's attempt to is what produced false "stopped" reports over live installations.

### Interruption and retry

On launch, `lib.rs` calls `tracker.reconcile(installed_now)`. A record whose phase is
active and whose pid is not ours describes a crash, a forced close, or a Windows restart.
It is reported as `Interrupted` with wording that depends on whether the runtime is
actually installed — **the stored flag is never restored as "running", and nothing is
relaunched automatically**. Each stage is already idempotent (`provision.rs` module docs),
so "Continue setup" resumes rather than repeating finished work.

Retry safety: the operation slot is claimed synchronously in the command, so a second click
is refused before it can start anything; released by `ThreadOperation`'s `Drop` on every
worker exit path, and explicitly if the thread cannot be spawned at all.

One honest limitation: release builds set `panic = "abort"`, so a panic in the worker ends
the process rather than unwinding, and `Drop` does not run. That is not a stuck lock — the
app is gone, and the next launch reconciles — but the guard's panic-safety applies only to
debug and test builds.

---

## 5. Diagnostics before changes

Unchanged and already correct, so it was reused rather than duplicated:
`runtime::health::preflight` runs before any modification and covers Windows version,
architecture, virtualisation, WSL availability and version, an existing distro, a genuine
pending restart, free space on **both** destination drives (15 GB when importing, 1 GB
otherwise), workspace creation and writability, path translation, and localhost binding.

**Offline installs are not blocked.** I verified there is no connectivity check anywhere in
`run_system_check` or `provision::run_setup`, and the install reads only the local package.
An offline machine with a valid bundled runtime installs normally.

What I did **not** do: extend diagnostics, add new blocker screens, or re-run preflight
after elevation. Preflight already runs on every `run_setup` call, so "Continue setup"
after a restart re-checks; but there is no re-check _within_ a single run between elevation
and import. That is a real gap and it is listed in §7.

---

## 6. Verification — exact commands and results

Run from the repository root on 2026-09-21.

```
npm run format:check       FORMAT_CHECK=0
npm run format:rust:check  FORMAT_RUST_CHECK=0
npm run typecheck:tools    TYPECHECK_TOOLS=0
npm run build              BUILD=0
npm run lint               KNIP=0
npm run lint:rust          CLIPPY=0      (-D warnings)
npm run test:rust          TEST_RUST=0   235 passed; 0 failed; 7 ignored
npm run test:ui            TEST_UI=0     69 passed
```

Baseline before this change: 209 Rust tests, 55 UI tests. Net **+26 Rust, +14 UI**.
All documentation links were re-checked and resolve.

### Evidence class — read this before trusting the numbers

| Claim                                                                                    | Evidence                                                                                       |
| ---------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| Navigation, reconnection, ordering, phase display, explainer, interruption reporting     | **Mocked UI tests.** Real React, real routing, real event plumbing, against a fixture backend. |
| Tracker state machine, sequence monotonicity, step accounting, reconciliation, redaction | **Real Rust unit tests.**                                                                      |
| Elevated report parsing, operation-id matching, pid parsing, PowerShell quoting          | **Real Rust unit tests on the pure functions.**                                                |
| The elevated install actually working end to end                                         | **Not tested.** See §7.                                                                        |
| Clean-Windows behaviour                                                                  | **Not tested.** See §7.                                                                        |

The fixture was made faithful on purpose — `run_setup` returns a snapshot and never
resolves with an outcome, and the mock refuses overlapping runs — so a passing UI test
exercises the real contract. It is still a mock.

### One assertion I had to fix, worth knowing about

`await expect(panel).not.toContainText("Checking your PC")` **passed vacuously**: every
stage name also appears in the step list, so the assertion could never fail. It is now
scoped to `panel.locator("h3")`, the only element that says which stage is current.

I then verified the ordering test is not vacuous by temporarily inverting `isNewer` to
`return true` and re-running it:

```
Expected: "Testing SageMath"
Received: "Checking your PC"
1 failed
```

The guard was restored immediately. **Any future change to the sequence rule should be
validated the same way** — this suite has now produced a vacuous assertion twice (see
`docs/reviewer/TESTING.md` for the earlier hover-hidden-button case).

### Two defects the green suite did not catch

Both were found by rendering the new screens and **looking at them**, after every test
passed. They are recorded here because they are the argument for doing that at all.

1. **The explainer dialog grew off the top of the screen.** `.dialog` had no `max-height`,
   which had never mattered because every previous dialog was short. Mine is long, so its
   title and the whole step list were pushed above the viewport with no way to scroll back.
   Fixed generally — `.dialog` is now a flex column capped at `calc(100vh - 48px)`, with
   only `.dialog-body` scrolling — which also protects the restore-backup dialog when a
   backup contains many workspaces.
2. **The "Set up SageDock" card stayed on screen beneath the live progress panel**, showing
   a disabled start button and a _second_ "What happens during setup?" next to the one in
   the panel. Two controls for one thing, one of them dead. The card is now hidden while a
   run is active and returns for a failed or interrupted one, which is when the retry
   button is wanted again.

Fixing (2) broke `clicking Set up twice does not start two installations`, which asserted
the button was _disabled_. Rather than relax it to "the button is gone" — which would have
proved much less — it now also invokes `run_setup` directly, behind the UI, and requires
the backend's operation lock to refuse it with `APP_BUSY`. A tidy UI is not the protection;
the lock is.

The remaining screenshots are `docs/qa/setup-running-light.png` and
`docs/qa/setup-explainer-light.png`, regenerated by test 68.

---

## 7. What remains untested, and how to test it

Ordered by how much risk it carries.

### 7.1 The elevated install has never been executed

**Nothing in this change exercised a real UAC prompt.** This machine already has WSL and
the VM platform enabled, so `install_wsl_elevated` is never reached. The supervisor, the
new PowerShell helper, the result file, and the pid liveness check have been tested only as
pure functions.

To test: a Windows VM with WSL and Virtual Machine Platform **off**, with a checkpoint
taken first. Then, as separate runs:

1. **Normal grant.** Expect `AwaitingConsent` → `HelperRunning` → `restart_required` or
   `completed`. Confirm the step list advances and elapsed time keeps moving.
2. **Decline the prompt.** Expect a warning, the app usable, and setup startable again.
3. **Leave the prompt unanswered for 10 minutes.** Expect the consent timeout, phrased as
   "nobody answered", with nothing installed.
4. **Kill the elevated helper** (`taskkill /PID <pid> /F` from an admin prompt) mid-install.
   Expect "ended without reporting a result" within ~10 s, naming the pid. **Verify no
   orphan `dism` survives**, and that the app does not claim the install stopped if one does.
5. **Close SageDock mid-install**, reopen. Expect `Interrupted`, no automatic restart of
   anything, and "Continue setup" resuming.
6. **Restart Windows mid-install.** Same expectations after reboot.

### 7.2 Clean-Windows compatibility

Not established, and this repository's standing rule is not to claim it without testing it.
An already-configured WSL machine proves nothing here.

### 7.3 Stages I did not put behind new tests

- **Insufficient storage / partially completed install.** `require_space` is unit-tested,
  but not the behaviour of a disk that fills _during_ import.
- **Corrupt runtime package.** `image::inspect` verification is tested; the setup path's
  response to a mid-import corruption is not.
- **Failed kernel verification.** There is an existing UI test for a failed verification
  message, but no integration test that a genuinely broken kernel is caught. The
  verification code itself is unchanged by this work.
- **Network loss.** Not applicable to the bundled-runtime path, which needs no network. It
  would matter only if an online installer is added.

### 7.4 A re-check gap I introduced no fix for

Between elevation completing and the runtime import, setup re-tests `wsl::is_available()`
and `vm_platform_present()` but does not re-run full preflight — so, for example, disk
space consumed by another process during a long elevation is not noticed until import
fails. Pre-existing behaviour, not a regression, but it is the kind of thing this work was
meant to surface.

---

## 8. Things an auditor should push on

- **`ThreadOperation` holds an `AppHandle` and calls `state::<AppState>()` in `Drop`.** If
  Tauri ever tears down managed state before worker threads finish, this would panic during
  unwind. It does not today; it is worth a second opinion.
- **The 500 ms supervisor poll calls `tasklist.exe`.** That is a process spawn twice a
  second for the duration of a Windows feature install. It is cheap relative to `dism`, but
  a native `OpenProcess` check would be cheaper and is the obvious follow-up.
- **`process_is_running` returns `true` when it cannot ask.** Deliberate — being unable to
  query is not evidence of death, and the patient direction is the safe one — but it does
  mean a broken `tasklist` turns the supervisor into an unbounded wait.
- **Redaction is a blunt filter over text SageDock writes**, not a general sanitiser. It
  drops whole lines naming a token and rewrites `\Users\<name>`. It is not relied upon to
  make untrusted third-party output safe.
