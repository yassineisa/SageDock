# Compilers, capability display, launcher, and icons

Handoff for an independent audit. Source version remains **1.3.1**: no version bump and no
release are part of this work.

Five changes: make the compiler and build-tool options genuinely functional, surface
capabilities on Home without side effects, make the Home button open JupyterLab's actual
landing page, give each tool a meaningful icon, and cover all of it with tests.

Read this alongside [reviewer/TESTING.md](../docs/reviewer/TESTING.md), which carries the current
test counts and states plainly what they do and do not establish.

## 1. What was actually wrong

Verified by reading the code before changing it, not assumed from the previous handoff:

- **`catalogue()` decided everything with `shutil.which()`.** A command on `PATH` was
  treated as a working compiler. A `gcc` with no C library headers, or a `g++` with no
  standard library, passed.
- **"Build tools" probed only `cmake`.** `make` and `pkg-config` were installed but never
  checked, so a group that was two-thirds missing reported as installed.
- **There was no "install all"**, despite `app.md` §11.1 specifying one.
- **Status was a boolean.** No distinction between "not installed", "installed but broken",
  "still checking", and "could not check".
- **Every status query started the Linux environment.** `catalogue()` calls `wsl::run_as`,
  which starts the distribution. Any capability display on Home would therefore have undone
  a deliberate Stop on every visit.
- **The Home button opened the file browser, not the launcher.** It called
  `open_notebook("")`, and `notebook_url` builds `/lab/tree/<path>`; an empty path yields
  `/lab/tree/`, which is the file browser at the server root. It also routed through
  `require_active_workspace()`, so after launching a course the general button silently
  became that course's button.
- **Every tool card used the same generic plus icon.**

## 2. Architectural decisions

**Verification means compiling and running a program.** `scientific.rs` carries a fixed
Python probe that, inside the runtime SageDock actually uses, builds and executes a small C
program, a C++ program (using `<string>` and a lambda, so the standard library is exercised,
not just the driver), and a Fortran program. Build tools are verified by doing their real
job: Make runs a shell rule, CMake configures a `project(... NONE)` — deliberately a
no-language project, so "is CMake working" stays a question about CMake and not about
whether a compiler happens to be present — and pkg-config resolves a `.pc` file written for
the purpose. Everything runs in a temp directory that is removed in a `finally`.

**Components, not tools, are the unit of truth.** `Component` is one executable;
`Tool::components()` maps a card to its components; `state_of()` derives `Installed` /
`NeedsRepair` / `NotInstalled`. This is what lets the UI say "Make works, CMake is missing"
instead of one misleading tick, and it is why `NeedsRepair` exists as a distinct state.

**The gather/interpret split is preserved.** The probe is untested by construction; every
decision function beside it is pure and unit-tested. See TESTING.md.

**Reading status must never start the environment.** `report(data, force)` probes only when
the distribution is already running, or when the user explicitly presses "Check now".
Otherwise it returns the last verified result from `tool-status.json`, labelled `Cached`, or
`Unavailable` when nothing is known. `ReportSource` is a tri-state precisely so the UI can
refuse to render "I could not look" as "it is not installed". `repair.rs` clears the cache
when the runtime is replaced, because those compilers no longer exist.

**Install skips completed work.** `missing_packages()` filters out components that already
verify, so "Install all" on a machine with a C compiler installs only what is missing, in
one apt transaction. Package names come from a closed allowlist keyed by `Component`; a test
asserts the set is closed. `dpkg --configure -a` runs first to recover an interrupted
install. `apt-get install` is given explicit package names with `--no-install-recommends`,
never a dist-upgrade.

**There is deliberately no cancel button.** Interrupting apt mid-transaction is how a
package database ends up half-written. The existing operation lock already prevents a second
install starting alongside one in flight.

**Removal is not offered anywhere, on purpose.** apt-removing `build-essential`, `gcc` or
`gfortran` can take shared libraries and headers with it that SageMath's own packages depend
on. Since dependency preservation cannot be proven here, the honest answer is not to offer
the action rather than to offer it and hope. Verify and Repair are offered instead.

**The launcher is a separate route and a separate command.** `jupyter::lab_url()` builds
`/lab`; `notebook_url()` still builds `/lab/tree/<path>`. `ViewerTarget` makes the two
intentions explicit at the call site. `open_jupyter_home` is rooted at
`state.paths.workspace_dir`, not the active workspace, so the general action stays general;
workspace cards still pass `ViewerTarget::Path("")` and open their own folder. The root
still bounds the server: "no specific path" means the top of the SageDock folder, never the
filesystem.

**Navigation is avoided when it would destroy work.** Reloading a notebook window discards
unsaved editor state and detaches running kernels from their views. `open_in_viewer` now
navigates only for a specific, non-empty notebook path; the launcher and a bare folder raise
the existing window instead. That also stops windows and servers accumulating.

**Icons were verified, not guessed.** Segoe Fluent Icons codepoints are private-use, so a
wrong one renders as an empty box. Each candidate was rasterised in Edge and compared
against a deliberately unassigned control codepoint to prove it was a real, distinct glyph,
then the shortlist was rendered at 16px and 32px in both themes and inspected before
choosing. The four chosen are `` braces for source code, `` ruler and set square
for numerical work, `` wrench and screwdriver for build tools, and `` toolbox
for the complete kit. They are written as escapes rather than pasted glyphs, because a
private-use character that degrades to a box during a copy is exactly the failure the
codepoint table exists to prevent.

## 3. Changed files

**New:** `docs/TOOLS-LAUNCHER-HANDOFF.md` (this file), `docs/qa/tools-light.png`,
`docs/qa/tools-dark.png`, `docs/qa/tools-minimum-light.png`.

**Rewritten:** `src-tauri/src/scientific.rs` (137 → 1099 lines), `src/pages/Tools.tsx`.

**Rust, modified:** `src-tauri/src/desktop.rs` (`ViewerTarget`, `open_jupyter_home`, both
tool commands), `src-tauri/src/lib.rs` (registers the new command), `src-tauri/src/home.rs`
(workspace launch call site), `src-tauri/src/jupyter/mod.rs` (`lab_url` plus a test),
`src-tauri/src/runtime/repair.rs` (clears the tool cache), `src-tauri/src/e2e.rs` (launcher
assertions plus the new integration test).

**Frontend, modified:** `src/pages/Home.tsx` (capability section, launcher command),
`src/lib/commands.ts` (tool types, three command signatures),
`src/components/Icon.tsx` (four icons), `src/styles/layout.css` (tool state, component rows,
capability chips).

**Tests:** `tests/ui/workspace.spec.ts` (mock rewritten for the component-level contract;
nine tests added, two updated).

**Docs:** `README.md`, `docs/DESIGN.md`, `docs/HOME-LAUNCH-AUDIT.md` (marked superseded
where it describes the old launcher), `docs/reviewer/TESTING.md`,
`docs/reviewer/ARCHITECTURE.md`.

**Untouched on purpose:** the logo, the About page and attribution placement, the restart
detection and its boot-timestamp fix, backup and workspace logic, and the version.

## 4. Test results

Every command below was run at the end of this work, on the current source:

| Check                  | Result                                 |
| ---------------------- | -------------------------------------- |
| `format:check`         | clean                                  |
| `format:rust:check`    | clean (after applying `cargo fmt`)     |
| `lint` (Knip)          | clean                                  |
| `lint:rust` (Clippy)   | clean with `-D warnings`               |
| `typecheck:tools`      | clean                                  |
| `build` (tsc + vite)   | clean                                  |
| `test:rust`            | **185 passed, 0 failed, 6 ignored**    |
| `test:ui` (Playwright) | **25 passed**                          |
| Integration runner     | **not run** — see the limitation below |

The Rust suite gained 17 passing tests: 16 in `scientific.rs` and one for the launcher URL.
The UI suite gained nine.

Four UI tests failed on their first run. Both causes were defects in the tests, not the
product, and are recorded here because they shape how the file should be read: `.tool-card`
filtered by `hasText` also matched the complete-toolkit card, whose component rows contain
every other tool's label, so cards are now located by their heading; and the state word
shares an element with the icon and version, so `toContainText` replaced exact text matching.

## 5. Screenshots

Regenerated by the suite and **inspected, not just produced**:

- `docs/qa/tools-light.png`, `tools-dark.png` — the four compiler cards with distinct icons,
  per-component rows, and the four state tones in both themes.
- `docs/qa/tools-minimum-light.png` — the page at the 860px minimum window width.
- `docs/qa/workspace-light.png`, `workspace-dark.png`, `home-minimum-dark.png` — Home with
  the capability strip.

Opening them caught one real defect that all 25 tests passed over: the "Verify" action was
`btn-subtle`, which renders as bare text, so on a card sitting beside bordered "Install" and
filled "Repair" buttons it read as non-interactive. It is now a normal button. This is the
second time in this repository's history that reading the renders caught what the suite
could not; do not skip it.

## 6. Limitations — read before trusting any of this

- **No compiler has ever actually been installed by this code.** The new integration test
  `compilers_install_and_really_build_a_program` exists and is wired into the runner, but it
  has not been executed: it needs a live WSL 2 distribution and downloads real packages. The
  Scientific tools page should be treated as implemented and unit-tested, **not**
  demonstrated. Run
  `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-fresh-install.ps1`
  against a throwaway QA distribution before relying on it.
- **The apt failure paths are classified but unexercised.** Lock contention, network
  failure and out-of-space are matched by substring against apt output and unit-tested with
  synthetic strings. No real failure has been observed, and apt's wording varies by version.
- **The UI tests are mocked.** They run in Edge against a fake IPC bridge, never WebView2.
  The mock derives group state from components the same way Rust does, which keeps it
  honest, but it is still a mock: it proves the page's wiring, not the backend's behaviour.
- **The visible JupyterLab landing experience is not verified.** The URL is now `/lab` and
  the integration test asserts that route serves the JupyterLab application, but JupyterLab
  restores its previous tab layout from its own saved workspace. A returning user may
  therefore see their last open tabs rather than the Launcher. Resetting that layout was
  deliberately **not** done, because it would discard a student's open notebooks to show a
  launcher. Whether the result is acceptable needs a human looking at a real session.
- **Progress during apt is coarse.** Three stages are emitted — checking, installing,
  verifying — with no percentage, because apt's output is not parsed. The UI shows an
  indeterminate bar rather than inventing a number.
- **`tool-status.json` is trusted as written.** It is only a cache and is cleared on runtime
  replacement, but a stale or hand-edited file would be shown as `Cached` until the next
  verified probe.
- **Python package installs were left as they were.** Seaborn, Statsmodels and Polars still
  use the existing additive `--target` plus `.pth` mechanism and an import check, which was
  outside this task's scope.
