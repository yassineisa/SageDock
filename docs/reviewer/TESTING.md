# What is tested, and what is not

Written for a reviewer deciding how much confidence the test suite actually earns. The
counts below were produced by running the suites at version **1.4.4**, not copied from an
earlier document.

## Current results

| Suite                  | Command                          | Result                               |
| ---------------------- | -------------------------------- | ------------------------------------ |
| Rust unit              | `npm run test:rust`              | **235 passed, 0 failed, 7 ignored**  |
| Mocked UI (Playwright) | `npm run test:ui`                | **69 passed**                        |
| Real integration (WSL) | `scripts/test-fresh-install.ps1` | 5 passed, **not re-run since 1.2.0** |

The tree contains 242 `#[test]` functions; 7 carry `#[ignore]` and are the integration
tests, which is why the default run reports 235 passed and 7 ignored. Total across all
three suites: **311**.

The setup-reliability work added 26 Rust tests and 14 UI tests. What they do and do not
establish — in particular that **the elevated install path has never been executed** — is
set out in [SETUP-RELIABILITY-HANDOFF.md](../SETUP-RELIABILITY-HANDOFF.md).

The integration row is the one to read carefully. It last ran at source version 1.2.0.
Everything added since — the compiler installation test, the JupyterLab launcher test, and
every behaviour introduced in 1.3.x and 1.4.x — has **never been executed against a real
WSL distribution**. Treat those features as implemented-and-unit-tested but not
demonstrated end to end.

## How the Rust tests are structured

The pattern to understand before reading any check module is the **gather/interpret
split**, described in the module comment of
[`src-tauri/src/system/mod.rs`](../../src-tauri/src/system/mod.rs):

- `gather()` performs the actual registry read, WMI query or process spawn. It is **not**
  unit-tested, because its result depends on the machine.
- `interpret(...)` is a pure function from gathered data to a `CheckItem`. It carries the
  decision logic and **is** unit-tested exhaustively.

This is why a module like `system/reboot.rs` can have ten tests without any of them
touching the registry. When reviewing a check, read `interpret` for the logic and treat
`gather` as the untested boundary.

The same split governs the two modules added in 1.4.3. `browsers.rs` unit-tests
`executable_from_command`, the pure parser that pulls a program out of a registered shell
command, and does not test the registry enumeration around it. `downloads.rs` unit-tests
the record's own behaviour — ordering, de-duplication, the cap, pruning files that have
gone, and refusing to overwrite a corrupt or newer-format list — and does not test the
WebView2 download event that feeds it.

Test counts by module, highest first:

```
workspaces.rs 21   backup.rs 21   setup.rs 18   runtime/wsl.rs 18
scientific.rs 17   runtime/provision.rs 17   runtime/image.rs 13   state.rs 12
system/reboot.rs 10   runtime/workspace.rs 10   downloads.rs 9   jupyter/notebook.rs 8
jupyter/mod.rs 7   system/wsl.rs 6   system/virtualization.rs 6   system/types.rs 6
library.rs 6   e2e.rs 6   config.rs 6   browsers.rs 6
system/windows_version.rs 4   system/disk_space.rs 4   system/architecture.rs 4
system/process.rs 3   system/mod.rs 1   storage.rs 1   home.rs 1   desktop.rs 1
```

The concentration in `workspaces.rs`, `backup.rs` and `provision.rs` is deliberate: those
are the three modules that can lose a student's work or leave an installation half-built.

`setup.rs` and the elevation half of `runtime/wsl.rs` joined them in the setup-reliability
work, for the same reason: between them they decide whether an installation can be left in
a state nobody can get out of. Note the split in what those 18 `wsl.rs` tests cover —
report parsing, operation-id matching, process-id parsing and PowerShell quoting are all
pure and genuinely tested; `install_wsl_elevated` itself is the untested boundary, and it
has never run.

`scientific.rs` follows the same gather/interpret discipline. Its tests all exercise pure
functions — `state_of`, `missing_packages`, `classify_apt_failure`, `assemble` — against
synthetic component results. None of them installs anything or touches WSL. That is a
deliberate boundary, and it is exactly why the integration test matters: the unit tests
prove the _decisions_ are right, not that a compiler was ever installed.

## The seven ignored tests

These are real integration tests. They are `#[ignore]` because they need a working WSL 2
installation and the staged runtime image, so they must never run as part of a normal
`cargo test`.

Six live in [`src-tauri/src/e2e.rs`](../../src-tauri/src/e2e.rs):

1. `installs_a_package_and_opens_a_sage_notebook` — full fresh installation through to an
   executing notebook. Also checks that Home's launcher serves JupyterLab's landing page
   from the default folder, reuses an existing server, and creates no files.
2. `installed_environment_reports_sage_and_kernels` — SageMath version and both kernel
   registrations.
3. `workspace_paths_with_shell_characters_survive` — the regression test for the
   no-shell-concatenation rule.
4. `windows_workspace_is_visible_inside_linux` — the Windows-to-Linux path bridge.
5. `compilers_install_and_really_build_a_program` — installs the C/C++ compiler for real,
   requires every component to compile _and run_ a program, checks a second install does
   not repeat the work, then stops the environment and confirms that reading the tool
   status returns the cached result without restarting it.
6. `jupyter_launcher_preserves_unsaved_notebooks` — launching from Home must not discard
   work open in an existing session.

And in [`system/mod.rs`](../../src-tauri/src/system/mod.rs):

7. `smoke_check_runs_against_this_machine` — runs every real diagnostic against the host
   and prints the result. This is the one test that deliberately touches the OS.

Run them with
`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-fresh-install.ps1`. The
runner imports the pinned image into a randomly named `SageDockQA-<guid>` distribution,
runs the others against it, restores the caller's environment variables even on failure,
and validates the registration's name and install path before unregistering it. Cleanup
failure fails the runner. Artifacts stay under `test-results/`.

Test 5 installs real Debian packages inside that throwaway distribution. It never touches a
student's own environment: `distro_name()` asserts the override starts with `SageDockQA-`,
and the override only compiles under `#[cfg(test)]`.

**The last integration run was at source version 1.2.0** (recorded in
[HOME-LAUNCH-AUDIT.md](../../coding%20agent%20onboarding/HOME-LAUNCH-AUDIT.md): 5 passed,
290.65s). Tests 5 and 6 did not exist then and have never run at all. No integration run
has happened since, so the compiler installation, the real JupyterLab landing page, and the
apt failure paths are all unexecuted.

## The 69 UI tests

`npm run test:ui` starts the Vite dev server on `127.0.0.1:1420` and drives it with
Playwright in **installed Microsoft Edge** (`channel: "msedge"` in
[`playwright.config.ts`](../../playwright.config.ts)), with the Tauri IPC layer mocked.

All 69 live in [`tests/ui/workspace.spec.ts`](../../tests/ui/workspace.spec.ts):

1. workspace, notebook creation, search, and navigation
2. new computer gets a clear setup path
3. the environment can be stopped from Home, with a warning to save first
4. backup restore and folder management work before SageMath setup
5. workspaces can be created, renamed, and removed without deleting files
6. restoring a backup previews its contents and never overwrites existing work
7. theme, guides, about, and recovery confirmation work by keyboard
8. diagnostics and compact window have no horizontal overflow
9. the Home screen has no horizontal overflow at a narrow width
10. captures Settings and Home in both themes, including the minimum window size
11. captures the setup state in dark theme
12. attribution appears only on About, not on every screen
13. a settings IPC failure is visible while the app continues with defaults
14. the attribution list holds together at the minimum window size
15. Home opens the JupyterLab launcher, not a notebook or a course folder
16. a workspace card opens its own folder rather than the launcher
17. JupyterLab startup failure is visible and the main button can retry
18. each build program is reported separately, and a partial group offers repair
19. Install all installs the whole toolkit in one step
20. a failed verification is explained and the retry succeeds
21. an unavailable status is never shown as 'not installed'
22. Home shows unknown tool status and never wakes a stopped environment
23. Home reflects a compiler becoming available after it is installed
24. compilers are never advertised as notebook languages
25. the tools page keeps distinct icons and holds at the minimum size in both themes
26. a workspace is renamed from the pencil beside its name
27. renaming is refused with a pop-up while a notebook is open in that workspace
28. files can be added to a workspace from its own card
29. Home lists recently opened and recently downloaded notebooks separately
30. clicking a downloaded notebook offers a choice of workspace, and a folder that isn't
    available can't be chosen
31. choosing a workspace with no conflict copies the download in, opens it, and makes that
    workspace active
32. a name already in Windows offers to open the existing notebook, keeping it untouched
33. replacing a colliding download overwrites the existing notebook under the same name
34. making another copy of a colliding download keeps both notebooks under different names
35. cancelling the name-collision dialog leaves the existing notebook untouched
36. a notebook can be dragged out of SageDock to Windows
37. an empty Downloads folder explains itself instead of showing a bare list
38. the Downloads list can be refreshed without reloading the rest of Home
39. the environment status and JupyterLab launcher are one card, not two
40. Recently Opened caps at six by default, and search still reaches the rest
41. hovering a Recently Opened row swaps the date for Delete, Show in File Explorer, and Open
42. deleting a recent notebook asks for confirmation and moves it to the Recycle Bin
43. the Downloads ribbon offers Delete, Add to workspace, and Show in File Explorer
44. deleting a download asks for confirmation and moves it to the Recycle Bin
45. Open notebook goes through the same workspace-and-conflict flow as a download
46. closing the Open notebook picker without choosing a file starts no workspace flow
47. a success notice clears itself after five seconds
48. a newly downloaded notebook is announced, and the notice auto-dismisses
49. a failed tool-status request leaves an actionable unknown status
50. a first run explains the app before asking for anything
51. skipping the introduction still counts as having seen it
52. an installation that has seen the introduction opens straight into the app
53. Settings can bring the introduction back, and opens the workspaces folder
54. the Settings browser picker appears only when notebooks open in a browser
55. a download saved outside the Downloads folder says which folder it went to
56. setup survives leaving Home and coming back
57. a screen mounting after setup already started recovers the full picture
58. a stale or out-of-order update cannot roll progress backwards
59. clicking Set up twice does not start two installations
60. a permission prompt is named as waiting for the user, not shown as working
61. waiting for Windows is distinguished from waiting for the user
62. a heartbeat is labelled as liveness rather than shown as progress
63. a failed setup explains itself and offers the next step
64. an interrupted run is reported on the next launch instead of silently resumed
65. a completed setup clears the busy state across every screen
66. What happens during setup explains this installation without leaving the page
67. the explanation stays available during setup and marks the running step
68. captures a setup run in progress and its explanation
69. the technical view and diagnostic export are available without leaving setup

Tests 56 to 69 cover the setup-reliability work. They exercise real React, real routing and
real event plumbing against a fixture that models the backend faithfully — `run_setup`
returns a snapshot immediately and never resolves with an outcome, the mock refuses
overlapping runs, and its state survives a page reload. They still prove nothing about a
real elevated install, which has never been executed; see
[SETUP-RELIABILITY-HANDOFF.md](../SETUP-RELIABILITY-HANDOFF.md) §7.

Tests 10, 11, 25, 50 and 68 double as screenshot generators, writing into
[`docs/qa/`](../qa/). Test 68 exists because assertions cannot see layout: reviewing its
output caught two defects the passing suite did not — a dialog taller than the window
growing off the top of the screen with no way to scroll back, and the "Set up SageDock"
card still sitting below the live progress panel, offering a dead button and a second copy
of "What happens during setup?". **Read the screenshots when changing these surfaces.**
Tests 8, 9, 14 and 25 assert no horizontal overflow at the 860px minimum window width
declared in `tauri.conf.json`.

Tests 18 to 25 cover the Scientific tools page and Home's capability strip. Read what they
actually prove: the mocked backend returns per-component results and the page derives state
from them, so they verify the _presentation and decision wiring_, including that an unknown
status is never rendered as "not installed" and that Home passes `force: false`. They do not
install anything, and no compiler exists in a Playwright run.

Tests 50 to 54 cover the first-run introduction added in 1.4.3. Because the introduction
**gates the whole app** — `Root` in `App.tsx` renders it instead of the router — the mock
reports it as already seen by default and the `showOnboarding` fixture option flips that.
Forgetting to do so fails every UI test at once rather than one.

### A trap this suite has already caught twice

Row actions in `NotebookRow` are `display: none` until the row is hovered or focused
(`layout.css`). `display: none` removes an element from the accessibility tree, and
Playwright's `getByRole` excludes hidden elements by default. An assertion such as
`getByRole("button", { name: /Delete/ }).toHaveCount(0)` therefore passes **whether the
button is genuinely absent or merely unhovered**, and proves nothing. Hover the row first;
tests 41 and 55 show the pattern.

## The validation gate

Run from the repository root. This is the full gate, and all of it passed at 1.4.4 except
the integration runner as noted above:

```powershell
npm run format:check
npm run format:rust:check
npm run lint              # Knip: unused files, exports, dependencies
npm run lint:rust         # Clippy with -D warnings
npm run typecheck:tools
npm run build             # tsc && vite build
npm run test:rust
npm run test:ui
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-fresh-install.ps1
```

Three practical notes that will otherwise cost you time:

- **Do not run two Rust builds at once on Windows.** The linker cannot replace an
  executable that is still running (`LNK1104`). Clippy, `cargo test` and `tauri build` must
  be sequential, and SageDock itself must be closed before a release build.
- The `--offline` Rust commands need a dependency cache already populated by one online
  build.
- **`npm run build` does not typecheck the tests.** It compiles `src/` only; the test tree
  is covered by `npm run typecheck:tools`. A syntax error in the spec can therefore show up
  as a green `build` alongside a Playwright run that reports _no tests found_. Read the
  per-command exit codes rather than a single aggregate one.

## What the suite does not establish

This is the part worth reading closely for a lab deployment decision.

- **The UI tests do not exercise WebView2.** They run in Edge against a mocked IPC bridge.
  They prove the React components' logic and layout, not that the packaged application
  renders or that IPC works in the real shell.
- **Native dialogs are unverified.** Every file and folder picker runs in Rust through
  `tauri-plugin-dialog` and is mocked out in tests.
- **The download Save As dialog has no automated coverage.** The `on_download` handler in
  `desktop.rs` shows an `rfd` save dialog and records where the file landed. Nothing in
  either suite can trigger a real WebView2 download. The developer has since exercised it
  by hand and reports it working, which is worth knowing but is not the same claim as
  tested: if it regresses, nothing here will catch it.
- **Installed-browser detection has no automated coverage either.** Only the pure
  command-line parser in `browsers.rs` is covered. The `StartMenuInternet` enumeration and
  the launch itself have likewise been confirmed by hand and are otherwise unguarded.
- **No clean-Windows verification exists.** Nothing here has been installed on a fresh
  Windows image lacking WSL. Windows-feature installation, UAC elevation, the restart and
  resume path, and virtualization-disabled recovery are covered only by unit tests over pure
  decision logic. The restart logic in particular infers a reboot by comparing a saved
  timestamp with `Win32_OperatingSystem.LastBootUpTime`; that comparison is unit-tested, but
  a real reboot transition has never been observed by a test.
- **Building an installer is not installing one.** The MSI is verified by reading its
  database back, not by running it.
- **A green suite has already proven insufficient once.** As recorded in
  [DESIGN.md](../DESIGN.md), four visual defects in the 1.1.3 redesign passed every test and
  were caught only by opening the screenshots: an accent below 4.5:1 contrast, phantom
  `auto-fill` grid columns, unreadable button labels mid theme-transition, and hover
  outranking selection. Open [`docs/qa/`](../qa/) rather than trusting the count.
