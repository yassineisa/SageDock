# Code-quality cleanup and handoff

For the subsequent direct JupyterLab launcher, plain workspace folders, restart audit,
and refreshed local test installer, see [HOME-LAUNCH-AUDIT.md](HOME-LAUNCH-AUDIT.md).
The earlier results below are historical; that handoff records the current validation.

Review date: 17 September 2026. Source version: **1.1.3**.

## Scope and evidence

Read `app.md` in full and inventoried the repository, separating owned source,
configuration, scripts, documentation, and assets from generated build output and
third-party dependencies. Removal decisions used import/reference searches, the Tauri
command registry and bundle configuration, TypeScript checks, Knip, and Rust compiler
and Clippy diagnostics. The repository has no commits, so there is no existing Git
baseline against which to produce a trustworthy before/after diff.

The current Metro/Fluent interface and logo are preserved. This pass changes source
and validation tooling; it does not rebuild the MSI or replace the bundled runtime.

## Deleted files

The following 16 files had no current consumer:

| Group                     | Files                                                                                                                                                                                                                                                    | Reason                                                                                                                  |
| ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| React components          | `src/components/EmptyState.tsx`, `src/components/HelpTip.tsx`                                                                                                                                                                                            | Neither was imported by the current interface.                                                                          |
| One-time source rewriters | `scripts/apply-backend-edits.cjs`, `scripts/edit-backend.py`, `scripts/finish-wiring.cjs`, `scripts/qa-prep.cjs`                                                                                                                                         | Obsolete migration scripts could overwrite the reviewed implementation; they are not build or maintenance tools.        |
| Store artwork             | `src-tauri/icons/StoreLogo.png`, `Square30x30Logo.png`, `Square44x44Logo.png`, `Square71x71Logo.png`, `Square89x89Logo.png`, `Square107x107Logo.png`, `Square142x142Logo.png`, `Square150x150Logo.png`, `Square284x284Logo.png`, `Square310x310Logo.png` | Unreferenced Windows Store assets; the configured installer target is MSI. All ten files were under `src-tauri/icons/`. |

The master icon and every icon declared in the Tauri bundle configuration remain.
Generated output and staged runtime files were excluded from source formatting.

## Updated components and behavior

- **Global footer:** added `AppFooter.tsx` to `AppShell.tsx`, showing the running
  binary's version immediately beside **Created by Yassin Eisa**. The version comes
  from `get_app_info`, with a readable fallback when that request fails. The footer
  uses the shared theme, stays below content on every route, and has an accessible
  landmark name. Both themes and the minimum supported window size are covered by UI tests.
- **Settings load failure:** `ConfigContext` already captured initial IPC errors,
  but nothing displayed them. The shell now explains when defaults are being used.
  Removed unused context fields while retaining the local system-theme resolution.
- **Frontend cleanup:** removed unused command wrappers, exported types, and CSS rules.
  Corrected the forced-colours workspace selector to target the current component.
  Production sources contain no redundant `console.log`, `console.info`, or
  `console.debug` calls. Rust tracing and integration-test progress output remain useful
  diagnostics and were retained.
- **Backend cleanup:** removed the unused IPC endpoints `run_system_check`,
  `stop_jupyter`, and `get_jupyter_status`, along with their unused frontend and
  response types. Internal diagnostic checks and guarded environment shutdown remain.
  Removed dead error builders, never-produced health variants, and obsolete Jupyter
  process-status helpers. These endpoints had no callers in the current frontend.
- **WSL detection:** diagnostics now use the runtime's existing distribution lookup
  and name instead of duplicating the registry enumeration and `SageDock` constant.
  This also honors the isolated distribution name in test builds.
- **Elevation result tests:** removed five tests of disconnected legacy helpers and
  replaced them with two tests of the exit-code classifier used by production
  installation. Success, restart-required, cancellation, missing status, and failure
  exit codes are covered. The default Rust total consequently changed from 158 to 155.
- **Architectural documentation:** clarified rollback ownership, backup validation and
  link handling, notebook path boundaries, task locking, runtime layering, and elevation.
  Removed outdated milestone comments and claims that WSL had never been exercised.
- **About attribution and project link:** About now lists the open-source components
  SageDock is built on alongside their licenses, and links the developer's source page.
  The address is a `const` in `commands.rs` reached through a new `open_project_page`
  command, so the webview cannot supply a URL and this stays one named operation rather
  than a general-purpose link opener. That distinction matters here: the JavaScript
  `opener` package and the `opener:default` capability were both removed above, and a
  plain `<a href>` in the main window would navigate the app away from itself, since only
  notebook windows carry an `on_navigation` guard. A new `.credit-row` pattern in
  `layout.css` carries the rows. `.check-row` could not be reused, because its
  `text-transform: capitalize` would have rewritten `pandas`, `scikit-learn`, and
  `conda-forge`, misattributing the projects. `THIRD-PARTY-NOTICES.md` now names the same
  components individually so the two lists agree.
- **Em dashes removed from user-visible text.** Seven characters across five passages in
  `Help.tsx` and `Home.tsx` became commas, full stops, or rewrites (two sat in a single
  paired construction on one line, and two more spanned a sentence across two lines). The
  seventeen that remain in `src/` are all inside source comments, which are not part of
  the interface. `DESIGN.md` records this as a language rule, together with the search
  that verifies it.

## Dependencies and formatting

- Removed unused direct dependencies `@tauri-apps/plugin-opener` (JavaScript) and
  `thiserror` (Rust), plus the unused `Win32_System_Threading` feature. The Rust opener
  plugin remains necessary for controlled desktop operations. `thiserror` still appears
  transitively in `Cargo.lock`; that is expected.
- Removed the unused JavaScript `opener:default` capability. Backend-managed folder and
  browser opening continue through the Rust plugin.
- Added pinned development dependencies for Prettier, Knip, and Node type definitions.
  Removed the obsolete Vite type-error suppression. Build configuration and Playwright
  tests now have their own strict TypeScript check.
- Added `.editorconfig`, Prettier configuration, and `knip.json`. Applied Prettier to
  supported owned source/configuration/documentation files and rustfmt to Rust source.
  `app.md`, generated output, runtime archives, and dependency directories are excluded
  from bulk formatting. Historical documentation received formatting, not a rewrite of
  its recorded results.
- Strict Clippy runs with warnings treated as errors. The one documented crate-level
  exception is `clippy::result_large_err`: structured IPC errors intentionally carry
  serializable user guidance and diagnostic details by value. No blanket dead-code or
  warning suppression was added. Other Clippy findings were fixed.

## Repeatable validation

Run from the repository root after installing dependencies:

```powershell
npm run format:check
npm run format:rust:check
npm run lint
npm run lint:rust
npm run typecheck:tools
npm run build
npm run test:rust
npm run test:ui
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-fresh-install.ps1
```

Use `npm run format` and `npm run format:rust` to apply formatting. The offline Rust
commands require a populated dependency cache. UI tests use installed Microsoft Edge
and mocked Tauri IPC; they do not prove native WebView2 or dialog integration.

The integration runner first imports the pinned image into a randomly named
`SageDockQA-*` distribution, then runs the other four ignored tests against that same
environment. It restores the caller's environment variables even on failure and checks
the registration's exact install path before unregistering it. Cleanup failures now
fail the runner. Test artifacts remain under `test-results/` for inspection.

Do not run Rust builds concurrently with the integration executable on Windows: the
linker cannot replace an executable that is still running.

## Results

- Production frontend build, tool/test TypeScript checks, and unused-code scan: passed.
- Strict Clippy, Prettier, and Rust formatting checks: passed.
- Default Rust suite: **155 passed, 0 failed, 5 integration tests separately selected**.
- Playwright: **14 passed, 0 failed**. Added checks for footer attribution/version
  alignment in both themes, for visible configuration-load failure, and for the About
  attribution list holding at the 860px minimum width while its project link actually
  reaches the backend. No other test covered the Help page at that width.
- PowerShell parsing of all three maintained scripts and Bash syntax checking of the
  runtime provisioning script: passed.
- Light and dark Settings screenshots were visually inspected; the footer is aligned,
  readable, and consistent with the existing layout. Updated evidence is in `docs/qa/`.
- Real integration suite: **5 passed, 0 failed**. The fresh-install and notebook test
  passed in **289.47 seconds**, reaching Ready after 251 seconds; the remaining four
  tests passed in **11.41 seconds**. Confirmed SageMath 10.9, Sage and Python kernel
  registration/execution, authenticated notebook access, separate workspace roots,
  shell-character path handling, Windows-to-Linux file access, and stop/restart.
  QA distribution cleanup succeeded. The OS diagnostic smoke test correctly reported
  the host's pending Windows restart as a warning; it did not prevent runtime checks.

Total: **174 passing tests** across the Rust unit, real integration, and mocked UI
suites.

The 155 Rust unit tests and 14 Playwright tests were re-run after the About attribution
change, together with rustfmt, strict Clippy, Prettier, Knip, and the tool TypeScript
check. The 5 real integration tests were **not** re-run for it: they need a live WSL
distribution, and the change adds a browser link and presentation only. Their result above
comes from the run recorded in this document, not from a later one.

## 1.2.0 release build

Built 17 September 2026 from this source, after the About attribution change. The gate above
was re-run at 1.2.0 **before** bundling, so the installer was not produced from an unverified
tree:

- `format:check` and `format:rust:check`: clean.
- `lint` (Knip) and `lint:rust` (Clippy, `-D warnings`): clean.
- `typecheck:tools` and `npm run build`: clean.
- `test:rust`: 155 passed, 0 failed, 5 ignored, out of 160.
- `test:ui`: 14 passed.

The release profile compiled in 3m 43s. The package was then verified by reading it back,
not by trusting its filename:

- `SageDock_1.2.0_x64_en-US.msi`, 1,469,779,968 bytes (1,401.7 MB).
- `ProductVersion 1.2.0`, `Manufacturer Yassin Eisa`, `ProductName SageDock`,
  `ProductCode {82664CD7-B326-41A2-B916-F014AB8BDBAE}`.
- Six file entries: `sagedock.exe` (7,917,568), the SageMath runtime archive
  (1,464,064,000, matching the size pinned in `trusted-runtimes.json`), its checksum
  manifest, `README.txt`, `LICENSE`, and `THIRD-PARTY-NOTICES.md` at 3,385 bytes, so the
  shipped copy carries the expanded attribution.

The embedded runtime at the pinned size is also what proves the `scripts/stage-runtime.ps1`
prerequisite in the README was already satisfied: `src-tauri/runtime/` and
`src-tauri/target/release/runtime/` both survived the repository cleanup, which removed only
`target/debug` and the redundant installer bundle.

The installer was **moved**, not copied, to `Desktop\SageDock-1.2.0-Installer.msi`. A
repository-wide search for `*.msi` afterwards returns zero results, so no gigabyte-scale
duplicate remains in the tree. Four older installers (1.0.1, 1.1.0, 1.1.1, 1.1.3) remain on
the Desktop and contain earlier implementations.

A version bump must change `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`,
`package.json`, and `package-lock.json` together. The lockfile carries the version twice, at
the root and under `packages[""]`, and npm metadata disagrees with the shipped package if
either is missed. Use full semver: Cargo and npm both reject a bare `1.2`.

## Attribution, logo, About page, and the restart warning

Four focused changes made after the 1.2.0 build, under an explicit instruction **not to
compile**. Nothing here has been built, type-checked, or executed. Read "Checks run and
checks deferred" below before trusting any of it.

### 1. Attribution made less prominent

The global footer is gone from every screen. `src/components/AppFooter.tsx` was **deleted**,
its render site and import removed from `AppShell.tsx`, and the `.app-footer`,
`.app-footer__version`, and `.app-footer__credit` rules removed from `layout.css`. No empty
strip remains: `.app-main` is a column flex container and `.app-content` carries `flex: 1`,
so the content reclaims the space with no layout change required.

Version, MIT licensing, and **Created by Yassin Eisa** now appear only on About.
Attribution in `LICENSE`, `README.md`, `THIRD-PARTY-NOTICES.md`, `Cargo.toml`,
`package.json`, and `tauri.conf.json` is untouched.

### 2. About promoted to its own navigation tab

New `src/pages/About.tsx` holds the version, developer credit, MIT statement, the
`github.com/yassineisa` link, and the twelve-row open-source attribution list. Registered at
`/about` in `App.tsx` and added to `NavRail.tsx` with the existing `info` glyph. The About
section was removed from `Help.tsx`, along with the `CREDITS` table, the `about`/`linkError`
state, the `openProject` handler, and the `commands`/`useEffect`/`AppInfo` imports that
became unused (Knip and `-D warnings` would otherwise flag them). Help keeps a
`btn-subtle` link to About so the route is still discoverable from where it used to live.

### 3. Logo recoloured

**There was no vector source.** The mark existed only as raster: `128x128.png`,
`128x128@2x.png`, `32x32.png`, `icon.png`, `icon.ico`, `icon.icns`. The instruction to "edit
the vector source directly" could not be followed as written, so one was authored:
**`src-tauri/icons/icon.svg`** now holds the canonical geometry — two open rings on a
diagonal, each with a concentric disc, each ring's gap facing the other.

Geometry, proportions, and silhouette are unchanged. Only the palette moved: cyan `#2DC9DE`
and amber `#FDBA2D` became `#C0304A` and `#D96A7C`. Those are **not** the UI accent
`#912338`, deliberately: against the dark theme's `#1c1b1a` that value reaches only about
2.6:1, under the 3:1 minimum for graphical objects, and one static asset has to work on both
backgrounds. Both chosen tones clear 3:1 on light and dark. Flat fills only.

`NavRail.tsx` now imports `icon.svg`, so the in-app logo is recoloured.

**The raster derivatives are NOT regenerated and remain cyan/amber.** No rasterizer is
available on this machine: `magick`, `inkscape`, and `rsvg-convert` are all absent. Note
that `convert` _does_ resolve on PATH, to `C:\WINDOWS\system32\convert` — the Windows
FAT-to-NTFS filesystem converter, not ImageMagick. It must not be invoked. `tauri.conf.json`
still points the installer icon at the old PNG/ICO/ICNS files, so **the next installer will
ship the old cyan icon** until someone runs, with ImageMagick installed:

```sh
magick -background none src-tauri/icons/icon.svg -resize 512x512 src-tauri/icons/icon.png
magick -background none src-tauri/icons/icon.svg -resize 32x32   src-tauri/icons/32x32.png
magick -background none src-tauri/icons/icon.svg -resize 128x128 src-tauri/icons/128x128.png
magick -background none src-tauri/icons/icon.svg -resize 256x256 src-tauri/icons/128x128@2x.png
npx @tauri-apps/cli icon src-tauri/icons/icon.png   # regenerates .ico and .icns
```

### 4. The persistent restart warning

**Root cause.** `system/reboot.rs` OR-ed three unrelated registry indicators into one
boolean and reported any of them as `ErrorSeverity::Warning`, which `Diagnostics.tsx` renders
as `check-warn` with a warning icon on every run.

**Evidence, read live from this machine's registry:**

| Indicator                                                                      | State                   |
| ------------------------------------------------------------------------------ | ----------------------- |
| `Component Based Servicing\RebootPending`                                      | absent                  |
| `WindowsUpdate\Auto Update\RebootRequired`                                     | absent                  |
| `SYSTEM\CurrentControlSet\Control\Session Manager\PendingFileRenameOperations` | **present, 26 entries** |

Every entry belonged to a Lenovo updater (`C:\ProgramData\lenovo\UDC\Hosts\x64\Microsoft.IdentityModel.*.dll`).
No Windows update was pending at all, so the message "Windows has an update waiting that
needs a restart to finish" was simply false here. That value is written by any installer
calling `MoveFileEx(..., MOVEFILE_DELAY_UNTIL_REBOOT)` and persists until the next boot, so
it is true on a large share of healthy machines.

It was also confirmed that the read succeeds rather than silently failing: the value is
`REG_MULTI_SZ`, and winreg 0.55 (`types.rs:33`) accepts `REG_SZ | REG_EXPAND_SZ |
REG_MULTI_SZ` for `String`, joining entries with newlines. The indicator is live, not dead
code.

**A second, worse path.** `provision.rs` called the same OR-ed predicate immediately after
DISM. The same Lenovo noise could therefore divert a clean, successful installation into
`SetupOutcome::AwaitingRestart`.

**Corrected decision logic.** The three indicators are now separate (`RebootIndicators`), and
the decision uses observed capability rather than registry flags:

- A restart **blocks** SageDock only when setup recorded one itself
  (`PersistedSetupState.awaiting_restart`), or when `wsl.exe` answers while the Host Compute
  Service is absent — precisely the state that produced `HCS_E_SERVICE_NOT_AVAILABLE`. Only
  these report `Warning`.
- Windows servicing pending (CBS or Windows Update) reports **`Info`** with honest wording:
  worth doing, does not stop SageDock.
- `PendingFileRenameOperations` reports **`Info`** naming it as another program's scheduled
  update. Still reported, not suppressed.
- A machine with no WSL at all is **not** blocked; setup installs WSL.
- `provision.rs` now gates on `wsl::is_available() && wsl::vm_platform_present()` and no
  longer reads restart flags from the registry. Legitimate post-DISM restarts are preserved,
  because an inactive virtual machine platform is exactly that condition.
- Detection reads live state on every check and persists nothing of its own, so the warning
  clears on the next run once the blocking condition resolves.

Nothing modifies Windows Update state, deletes a registry value, or hard-codes a healthy
result.

`run_system_check` now takes `setup_awaiting_restart: bool`, supplied by `desktop.rs` and
`runtime/health.rs` from persisted setup state. `diagnostic_report` additionally emits
`overall`, which was previously computed and discarded — meaning `HealthState::RestartRequired`
had no UI consumer at all before this change.

### Changed and deleted files

**Deleted:** `src/components/AppFooter.tsx`.

**New:** `src/pages/About.tsx`, `src-tauri/icons/icon.svg`.

**Modified:** `src/components/AppShell.tsx`, `src/components/NavRail.tsx`, `src/App.tsx`,
`src/pages/Help.tsx`, `src/styles/layout.css`, `tests/ui/workspace.spec.ts`,
`src-tauri/src/system/reboot.rs` (substantially rewritten),
`src-tauri/src/system/mod.rs`, `src-tauri/src/system/types.rs`,
`src-tauri/src/desktop.rs`, `src-tauri/src/runtime/provision.rs`,
`src-tauri/src/runtime/health.rs`, `docs/DESIGN.md`, `docs/CODE-QUALITY.md`.

**Deliberately untouched:** notebook, workspace, backup, recovery, and runtime-isolation
logic; `LICENSE`; `THIRD-PARTY-NOTICES.md`; `README.md`; package metadata; the raster icons.

### Added and updated tests

**Rust, `system/reboot.rs`** — ten tests, up from two, eight of them against the pure `interpret`, split by the three
cases asked for:

- _False positives:_ a third-party scheduled file rename is `Info`; a pending Windows update
  alone is `Info`; a machine with no WSL is not reported as needing a restart.
- _Genuine blockers:_ features enabled but platform inactive is `Warning`; a restart recorded
  by setup is `Warning`; a blocking restart outranks advisory indicators.
- _Clearing:_ the warning becomes `Info` once the platform is present; a clean machine says
  "No restart is needed."
- _Predicate:_ `servicing_pending()` ignores file renames and honours CBS/Windows Update.

**Rust, `system/types.rs`** — `an_advisory_pending_reboot_leaves_the_overall_state_healthy`
guards the rollup, which previously escalated on any non-`Info` severity.

**Playwright, `tests/ui/workspace.spec.ts`** — the footer test was replaced by
`attribution appears only on About, not on every screen`, which asserts no `contentinfo`
landmark and no creator credit on `/`, `/#/settings`, `/#/recovery`, and `/#/help`, then
checks both are present on `/#/about`. The keyboard test now reaches About through the
navigation pane, and the minimum-width attribution test targets `/#/about`. The IPC-failure
test asserts the footer landmark is absent.

### Checks run and checks deferred

> **Superseded — read this before trusting the list below.** This section records the state
> at the end of that one task, when an explicit instruction forbade compiling anything.
> Everything listed here as deferred has since been executed: see
> [HOME-LAUNCH-AUDIT.md](HOME-LAUNCH-AUDIT.md) and [reviewer/TESTING.md](../docs/reviewer/TESTING.md)
> for current results. The `docs/qa/` screenshots have also been regenerated and no longer
> show the footer or the cyan logo.

**Run:**

- `npm run format` / `npm run format:check`: clean. Prettier parses TS/TSX, so this also
  confirms the frontend files are syntactically valid.
- `cargo fmt --check`: reached every file, which confirms the Rust **parses**. It reported one
  formatting difference (a stray blank line in `reboot.rs`), since corrected.
- Read-only registry inspection of all three reboot indicators, which produced the evidence
  above.
- Repository-wide searches for dangling references to `AppFooter`, `app-footer`,
  `reboot_is_pending`, `run_system_check`, and `128x128.png`. That search caught a call site
  in `runtime/health.rs` that the edits had otherwise missed.

**Deferred because compilation was prohibited:**

- `cargo check`, `cargo clippy -D warnings`, `cargo test` — so **none of the new Rust tests
  have been executed**, and type errors or dead-code warnings remain possible.
- `npm run build` and `npm run typecheck:tools` — no TypeScript type checking has run.
- `npm run test:ui` — no Playwright test has run, so the new and updated assertions are
  unverified and the `docs/qa/` screenshots are stale (they still show the footer and the
  cyan logo).
- Any visual inspection of the recoloured logo, the About page, or the footer-free layout.

### Remaining uncertainty

- **Highest risk: the Playwright selectors.** `getByRole("link", { name: "About", exact: true })`
  assumes the nav link's accessible name is exactly "About" and does not collide with Help's
  "About SageDock" button. Untested.
- `page.getByText(appVersion, { exact: true })` on About may match more than one element and
  throw a strict-mode violation; it was not run.
- The SVG renders correctly in principle, but no browser has loaded it. If the arc path data
  is wrong the navigation logo will look broken, and Vite's SVG asset import for
  `../../src-tauri/icons/icon.svg` is unexercised.
- Rust dead-code risk: `RestartContext` and `RebootIndicators` are `pub` inside a private
  module. I removed `any()` and a free `servicing_restart_pending()` for exactly this reason,
  but only Clippy can confirm none remains.
- Whether the two logo tones look right at 16px in the navigation rail is a judgement no
  contrast calculation settles.
- The root cause is proven on _this_ machine. A different machine showing the warning could in
  principle have a genuine CBS or Windows Update indicator instead, which the new logic
  reports as `Info` rather than a blocker — correct, but it means the symptom can have more
  than one source.

## Remaining release validation

The current machine already has WSL installed. A successful new QA distribution proves
runtime installation and notebook integration on this machine; it does not prove initial
Windows feature installation, UAC, reboot/resume, or virtualization-disabled recovery.
The clean-Windows VM, native dialog, fault-injection, and packaged WebView2 checks in
[QA-REVIEW.md](QA-REVIEW.md) remain necessary. The 1.2.0 installer recorded above was built
from the current source and supersedes the older artifacts, which predate this cleanup.
Building a package and reading it back is not installing it: the installation itself, and
the interface running in real WebView2 rather than mocked Edge, both remain untested.

## 1.3.1 release build

Built 18 September 2026. This is the first build in which the **full gate was executed
against the attribution/logo/About/restart change set** — the section above recorded those
changes as unverified because compilation was prohibited at the time. Results at 1.3.1:

- `lint` (Knip) and `lint:rust` (Clippy, `-D warnings`): clean.
- `typecheck:tools` and `npm run build`: clean.
- `test:rust`: **168 passed, 0 failed, 5 ignored**.
- `test:ui`: **16 passed**, which also regenerated `docs/qa/`.
- `format:check` and `format:rust:check`: clean.
- Release profile compiled in 3m 30s.

The five real integration tests were **not** re-run at 1.3.1. They need a live WSL
distribution, and the 1.3.1 changes are the version bump and the icon rasters, neither of
which touches runtime provisioning. Their last result is the one in
[HOME-LAUNCH-AUDIT.md](HOME-LAUNCH-AUDIT.md).

### Logo rasters finally regenerated

The earlier section recorded that the recoloured `icon.svg` had no regenerated derivatives,
because no rasterizer existed on this machine. That gap is now closed: **Playwright is
already a dev dependency and is configured against installed Microsoft Edge**, so Edge's
renderer rasterizes the vector source. `32x32.png`, `128x128.png`, `128x128@2x.png`,
`icon.png` and a six-entry `icon.ico` (16/24/32/48/64/256, PNG-compressed entries) were
regenerated from `icon.svg` and verified by decoding them back: the dominant opaque colours
are exactly `#D96A7C` and `#C0304A`, at ~30% opaque coverage with alpha preserved, which
also confirms the SVG's arc geometry renders correctly. The 512px render was inspected
visually. `icon.icns` is untouched and remains the old palette; it is macOS-only and
`bundle.targets` is `["msi"]`, so nothing consumes it.

Because the icons were regenerated at 00:17 and the bundle was produced at 00:25, this is
the first installer carrying the burgundy mark.

### Package verification

Verified by reading the MSI database back, not by trusting the filename:

- `SageDock-1.3.1-Installer.msi`, **1,469,722,624 bytes** (1,401.6 MB).
- `ProductName SageDock`, `ProductVersion 1.3.1`, `Manufacturer Yassin Eisa`.
- `ProductCode {BF9DF498-1798-458C-9099-A75CA4E47CC0}`,
  `UpgradeCode {B05BF660-072E-5795-9B7F-9C42C518C5E8}`.
- `ALLUSERS 1`, so this is a per-machine installation requiring administrator rights.
- Six file entries: `sagedock.exe` (7,888,384), the SageMath runtime archive
  (1,464,064,000, matching the size pinned in `trusted-runtimes.json`), its checksum
  manifest, `README.txt`, `LICENSE`, and `THIRD-PARTY-NOTICES.md`.

The installer was **moved**, not copied, to `Desktop\SageDock-1.3.1-Installer.msi`; a
repository-wide search for `*.msi` afterwards returns zero results.

### Repository size

The tree had grown back to **15 GB**, all of it regenerable build output. Removing
`target/debug`, the stale `1.2.0` bundle, the `0.1.0` NSIS artifact, the resource staging
copy of the runtime and the rust-analyzer scratch directory brought `target` from 12 GB to
1.8 GB, while preserving the release compile cache. `src-tauri/runtime/` (1.4 GB) was kept
deliberately: it is git-ignored but is a **build input**, and an installer build fails
without it.

**Expect this to regrow, and know which part is disposable.** Running the gate rebuilt
`target/debug` to **5.9 GB** (3.6 GB of `deps`, 598 MB of incremental dep-graphs, and a
1.4 GB resource staging copy of the runtime), taking the tree back to 9.2 GB. It was removed
again afterwards. Nothing in `target/` is source, so the safe reclamation after any test or
build run is:

```powershell
Remove-Item -Recurse -Force src-tauri/target/debug
Remove-Item -Recurse -Force src-tauri/target/release/runtime, src-tauri/target/release/bundle
```

Keeping `target/release/deps` avoids a full recompile; deleting `target/debug` only costs a
rebuild the next time Clippy or `cargo test` runs. Steady-state repository size with the
release cache and the staged runtime retained is **3.3 GB**, of which 3.2 GB is those two
regenerable-or-re-stageable items.

Note also that the repository lives inside a OneDrive-synced Desktop folder, so anything
left in the tree is a candidate for cloud upload. That is a further reason not to leave
gigabyte build output lying around.
