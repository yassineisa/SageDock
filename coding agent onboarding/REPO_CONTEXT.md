# SageDock repository context

Written for a coding agent or contributor who has never seen this repository and needs to
make a correct change without breaking something expensive. Read it start to finish; it is
long because the traps are specific.

**Source version when written: 1.4.4.**

---

## 1. What the product is

SageDock is a **Windows desktop application** that makes SageMath, Python and JupyterLab
usable by a student with no technical knowledge and no terminal. It installs and manages
its own **WSL 2 Linux distribution** containing SageMath, and keeps the student's notebooks
in ordinary Windows folders **outside** that distribution.

The original product specification is [`app.md`](app.md) (1,345 lines). It calls the
product _DataLab_; the name later changed to SageDock, and all code, identifiers and paths
use SageDock. Parts of it have been deliberately superseded, see
[`../docs/reviewer/README.md`](../docs/reviewer/README.md) for the currency table.

### The one invariant everything follows from

> The Linux runtime is **disposable infrastructure**. The student's notebooks are **not**.
> Repair, reinstall, reset and uninstall may destroy the former and must **never** touch
> the latter.

Most of the architecture is a consequence of this. If a change you are making blurs that
line, it is almost certainly wrong.

### Standing rules a change must not violate

These are operational constraints on the product, not style preferences:

- Never run the whole application elevated. Elevation is confined to Windows-feature
  installation during setup.
- Never disable Windows security protections.
- Never expose the notebook server on the network. It binds `127.0.0.1` only.
- Never concatenate untrusted input into a shell command. Everything is `argv` arrays.
- Never use `wsl --shutdown`, it affects unrelated distributions belonging to other
  coursework. Terminate only SageDock's own distro, by name.
- Never delete a user's project folders during uninstall or repair without explicit
  confirmation.
- Destructive actions must explain what will happen and require deliberate confirmation.
- Do not claim clean-Windows compatibility. It has never been tested (see §11).

---

## 2. Technology and shape

Tauri v2, a Rust binary owning a WebView2 window that renders a React 19 / TypeScript
frontend built by Vite.

```
sagedock.exe  (Rust: src-tauri/src/main.rs -> lib.rs::run)
├── window "main"              the React UI; capability: core:default only
├── window "notebook-<port>"   JupyterLab at an external URL; NO IPC capabilities
└── child process: wsl.exe --exec ... sagedock-jupyter   (one per open workspace)
```

Rough size: **14,702 lines of Rust** across 36 files, **6,648 lines of frontend** across
27 files, plus **2,571 lines** of UI tests in a single spec. (Measured at 1.4.4.)

`main.rs` is 6 lines and calls `run()`. **[`src-tauri/src/lib.rs`](../src-tauri/src/lib.rs)
is the best first file to read**: it registers plugins, resolves every path once into
`AppPaths`, installs the window-close/exit guards and the drag-drop handler, and lists
every IPC command in one block.

### Dependencies worth knowing

| Crate / package                | Why it is here                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tauri` v2 + `tauri-build`     | The shell, window and IPC layer.                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `tauri-plugin-opener`          | Open a URL/file/folder with the OS default. Callable from **Rust** without a capability grant.                                                                                                                                                                                                                                                                                                                                                                            |
| `tauri-plugin-dialog`          | Native file pickers, always run in Rust, never in the webview.                                                                                                                                                                                                                                                                                                                                                                                                            |
| `tauri-plugin-single-instance` | Registered **first**. One SageDock owns the environment; a second launch focuses the first and exits.                                                                                                                                                                                                                                                                                                                                                                     |
| `winreg`                       | Read-only registry access: Windows version, WSL registration, reboot flags, installed browsers.                                                                                                                                                                                                                                                                                                                                                                           |
| `ureq`                         | Used **only** against `127.0.0.1` for Jupyter readiness and shutdown.                                                                                                                                                                                                                                                                                                                                                                                                     |
| `zip` (deflate only)           | The portable backup container. Default features off so a backup opens in any standard tool.                                                                                                                                                                                                                                                                                                                                                                               |
| `drag`                         | Starts a native OS drag so a notebook can be dragged out to the desktop. Used instead of `tauri-plugin-drag`, which exposes the operation only as a webview command, that would mean handing absolute paths to the frontend.                                                                                                                                                                                                                                              |
| `trash`                        | Deleting sends to the Recycle Bin rather than unlinking, the same margin Explorer gives.                                                                                                                                                                                                                                                                                                                                                                                  |
| `rfd`                          | The Save As dialog for downloads from the built-in browser. Forced, not preferred: WebView2 raises the download event on the UI thread, where `tauri-plugin-dialog`'s `blocking_*` pickers deadlock the event loop by its own documentation, and its callback form answers after the handler must already have returned a destination. `rfd` is what that plugin wraps, so this adds no new transitive dependency; the version and features are copied from its manifest. |
| `getrandom`, `sha2`, `base64`  | Session tokens, runtime image checksums.                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| React 19, React Router 7       | Frontend. `HashRouter`, because the packaged shell serves from a file origin.                                                                                                                                                                                                                                                                                                                                                                                             |

There is **no telemetry, analytics or crash reporting** anywhere in the tree.

---

## 3. The frontend/backend boundary

All **48 IPC commands** are registered in one `generate_handler!` block in `lib.rs`:

| Module                                        | Count | Concern                                                                    |
| --------------------------------------------- | ----- | -------------------------------------------------------------------------- |
| [`commands.rs`](../src-tauri/src/commands.rs) | 13    | app info, config, browsers, onboarding, setup, notebook creation, shutdown |
| [`desktop.rs`](../src-tauri/src/desktop.rs)   | 18    | notebook opening, the launcher, downloads, tools, recovery, diagnostics    |
| [`home.rs`](../src-tauri/src/home.rs)         | 17    | environment status, workspaces, imports, backups                           |

On the frontend, [`src/lib/commands.ts`](../src/lib/commands.ts) is the **only** file that
calls `invoke`. Components import the typed `commands` object from it. This is a deliberate
auditing property: the complete set of operations the webview can trigger is one readable
list.

### Four boundary rules

1. **No command accepts an absolute path.** Notebook paths are workspace-relative and
   re-resolved by `library::resolve` on every use.
2. **Native pickers run in Rust.** A chosen file is held in `AppState`
   (`selected_package`, `selected_backup`, `selected_import`) between the pick and the
   confirmation, specifically so the path is never handed to the webview and passed back.
3. **No command accepts shell text or a URL.** `open_project_page` uses a Rust constant.
   `browsers::open` resolves a registry identifier to an executable in Rust.
4. **Downloads are named by an opaque key, not a path.** The Downloads list has two
   sources: files in the Windows Downloads folder, keyed by their bare name, and downloads
   the built-in browser recorded anywhere on disk, keyed by `tracked:<hash-of-path>`. A
   tracked key is only ever matched back against the recorded list
   (`library::resolve_download`), so it cannot name a file SageDock did not download
   itself, and the path never crosses the IPC boundary in either direction. Rows carry the
   containing folder's **name** for display, never its path.

### Events (Rust → UI, streamed rather than polled)

`setup-progress`, `backup-progress`, `tool-progress`, `close-requested`, `files-dropped`.

**`setup-progress` is not like the others.** It carries a whole `SetupSnapshot`, not a
delta, and the same snapshot is separately queryable via `setup_snapshot`. That is
deliberate: setup is owned by the backend and outlives any screen, so a screen must be able
to recover the full picture by asking rather than by having been listening. Every snapshot
carries `operation_id` and a monotonic `seq`, and **an observer must ignore any snapshot
that is not newer**, same operation and `seq` not greater. Without that rule a slow query
answering after a fast event rolls the display backwards. See
[`../docs/SETUP-RELIABILITY-HANDOFF.md`](../docs/SETUP-RELIABILITY-HANDOFF.md) §3.

### Argument casing

The `#[tauri::command]` macro defaults to `rename_all = "camelCase"`, so JS `workspaceId`
maps to Rust `workspace_id`. **The UI test mock intercepts `invoke` directly and therefore
sees the camelCase keys verbatim**, a mock keyed on `workspace_id` silently never matches.

---

## 4. Rust module map

### `runtime/`, owns installation and the Linux environment

Layering is intentional and stated in
[`runtime/mod.rs`](../src-tauri/src/runtime/mod.rs). Violating it is the main thing to watch
for in review.

| File           | Responsibility                                                                                                         |
| -------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `wsl.rs`       | **The only module that executes WSL commands.** Distro naming, import/export, terminate, unregister, path translation. |
| `image.rs`     | Finds and verifies the SageMath package against `trusted-runtimes.json`.                                               |
| `workspace.rs` | Prepares Windows folders; resolves Documents through a fallback chain.                                                 |
| `provision.rs` | The setup state machine. Stages are idempotent, so an interrupted setup resumes by running again.                      |
| `health.rs`    | `preflight` (launch checks) and `fast` (is the environment usable right now).                                          |
| `repair.rs`    | Replace or rebuild the environment. Never deletes Windows workspaces.                                                  |

`runtime::contract` pins the four paths every runtime image must provide, so a new image can
change SageMath versions without an app update.

### `system/`, read-only diagnostics

Nine modules, each one check, all **read-only**. Installation and repair belong to
`runtime/`. Every check follows the **gather/interpret split**:

- `gather()` does the registry read, WMI query or process spawn. **Not unit-tested**, its
  result depends on the machine.
- `interpret(...)` is a pure function from gathered data to a `CheckItem`. **Exhaustively
  unit-tested.**

Follow this split for any new check. `system/reboot.rs` is worth reading in full: its
module comment documents a real bug where three unrelated Windows "restart pending"
indicators were collapsed into one boolean, making SageDock demand a restart on nearly
every machine.

### `jupyter/`, the notebook service

`mod.rs` owns the server lifecycle: loopback binding, a per-session CSPRNG token, an
OS-assigned port, readiness by polling `/api/status` rather than sleeping, and graceful
shutdown that asks Jupyter to stop before killing `wsl.exe`.

**JupyterLab routing is fiddly and has caused a shipped bug.** The valid routes are:

| Route                                | Result                                                          |
| ------------------------------------ | --------------------------------------------------------------- |
| `/lab`                               | Landing page                                                    |
| `/lab/tree/<path>`                   | File browser at a path                                          |
| `/lab/workspaces/<name>`             | A named UI workspace                                            |
| `/lab/workspaces/<name>/tree/<path>` | Valid                                                           |
| `/lab/workspaces/<name>/tree`        | **Invalid**, client-side "Path Not Found", then redirect to `/` |

The last one returns **HTTP 200**; the failure is JupyterLab's client-side router, so
probing with HTTP alone will tell you everything is fine when it is not. `session_url` is
the single function that decides whether a `/tree` segment belongs in the URL. Do not
rebuild URLs by string surgery elsewhere, that is exactly what caused the bug.

### Application layer

| File            | Responsibility                                                                                                                     |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `backup.rs`     | Portable ZIP backup/restore of the student's work. Largest module. See [`../docs/BACKUP-FORMAT.md`](../docs/BACKUP-FORMAT.md).     |
| `workspaces.rs` | The workspace registry. A workspace is nothing more than an ordinary Windows directory.                                            |
| `scientific.rs` | Compilers and build tools: an allowlisted catalogue, verified by compiling **and running** test programs.                          |
| `library.rs`    | Notebook discovery, import, Downloads scanning, relative-path resolution, Recycle Bin deletion.                                    |
| `state.rs`      | `AppState`, the operation lock, running servers, the config mutex, the setup tracker.                                              |
| `setup.rs`      | The backend-owned setup operation: phases, the step plan, sequence numbering, crash reconciliation, log redaction.                 |
| `browsers.rs`   | Installed-browser detection from the registry, for the first-run choice.                                                           |
| `downloads.rs`  | A record of what the built-in browser downloaded, so Home lists a file saved outside the Downloads folder. A record, never a scan. |
| `config.rs`     | Settings persistence.                                                                                                              |
| `error.rs`      | `AppError` / `ErrorSeverity`.                                                                                                      |
| `storage.rs`    | `atomic_write`. Never used for notebooks, Jupyter owns those.                                                                      |
| `logging.rs`    | Structured JSON logging.                                                                                                           |

---

## 5. Cross-cutting patterns

**Every failure is an `AppError`.** It answers four questions: what happened
(`title`, `message`), is my work safe (`user_files_safe`), can SageDock fix it
(`recovery_actions`), and the technical detail (`technical_details`, shown behind a
disclosure). Commands must never return raw process output or bare strings.
`user_files_safe` **defaults to `true`**, check it is accurate on any new failure path.

**One long operation at a time.** `AppState::begin_operation` returns an `OperationGuard`
whose `Drop` clears the label, so an early return or a panic cannot leave the app
permanently busy. The label is a sentence fragment completing "SageDock is still …".

**One Jupyter server per workspace root**, capped at `MAX_LIVE_SERVERS = 6`. A server
exposes exactly one directory tree, so re-rooting a shared one would either kill a running
calculation or show the wrong folder.

**Atomic writes for app-owned JSON.** `storage::atomic_write` writes to a randomly named
temp file, `sync_all`s, then renames.

**Paths resolved once** into `AppPaths` in `lib.rs::run`. Note the Documents resolution
chain: on a machine with OneDrive-redirected Documents the known-folder API can return an
empty path, which would drop notebooks into AppData where nobody would find them.

**Two workspace locations, deliberately siblings:**

- `Documents\SageDock`, the default workspace, created by setup (`AppPaths::workspace_dir`).
- `Documents\SageDock Workspaces`, where workspaces the student creates live
  (`home::workspaces_parent`). A sibling, never a child: nesting would make every
  workspace's files appear inside the default workspace's file browser and be counted twice
  in a backup.

---

## 6. Frontend structure

```
src/main.tsx          mounts App, imports theme.css then layout.css
src/App.tsx           ConfigProvider > TaskProvider > Root
                      Root renders EITHER <Onboarding/> OR HashRouter+AppShell
src/components/
  AppShell.tsx        nav + content, task bar, error banner, notice, close dialog
  NavRail.tsx         7-entry navigation pane, collapses to icons below 900px
  Icon.tsx            Segoe Fluent Icons codepoint table, the ONE verified place
  WorkspaceCard.tsx   NotebookRow.tsx  ChoiceDialog.tsx  ConfirmDialog.tsx  ErrorBanner.tsx
src/pages/            Home (largest), Onboarding, Tools, Settings, Recovery,
                      Diagnostics, Help, About
src/state/
  ConfigContext.tsx   theme, browser and onboarding settings; tolerates a failed load
  TaskContext.tsx     the busy/error/message surface used by every page
  SetupContext.tsx    the ONE observer of setup, mounted above the router on purpose
src/styles/
  theme.css           tokens, type ramp, base elements
  layout.css          structure and components
```

**Styling rule, enforced in review: components carry class names only.** Colours and
spacing come from tokens in `theme.css`. There are no page-specific stylesheets. The full
rationale is in [`../docs/DESIGN.md`](../docs/DESIGN.md), which is **binding** for any UI
change.

### Settings shape

```ts
interface AppConfig {
  schema_version: number;
  theme: "system" | "light" | "dark";
  open_in_browser: boolean;
  onboarding_complete: boolean; // first-run introduction finished or skipped
  preferred_browser: string | null; // a Browser.id, or null for the Windows default
}
```

Every field is `#[serde(default)]` in Rust, so a settings file from an older version loads
cleanly. A missing or corrupt config is **never fatal**, it falls back to defaults and
logs a warning.

### The first-run introduction

`Root` in `App.tsx` renders `<Onboarding/>` **instead of** the whole shell when
`onboarding_complete` is false. Two consequences that are easy to get wrong:

1. **Anything gating the app must be reflected in the UI test fixture.** The mock's
   `get_config` returns `onboarding_complete: true` by default; the `showOnboarding`
   fixture option flips it. Forget this and all ~50 UI tests fail at once.
2. **When the config load fails, the fallback sets `onboarding_complete: true`.** Settings
   are unreachable in that state, so completion could not be saved and the introduction
   would reappear forever, a first-run screen nobody can get past is worse than none.

---

## 7. Security posture

Full claim-by-claim detail, each naming its implementing file, is in
[`../docs/reviewer/SECURITY.md`](../docs/reviewer/SECURITY.md). The short version:

- **Notebook server**: `--ServerApp.ip=127.0.0.1`, `allow_remote_access=False`, OS-assigned
  port, a fresh 32-byte CSPRNG token per session. `verify_kernels` positively asserts that
  an unauthenticated request is rejected.
- **No shell concatenation**: `argv` arrays through `wsl --exec`, which does not involve a
  Linux shell.
- **Capabilities**: `capabilities/default.json` grants exactly `core:default` to the window
  labelled `main`. Plugins are registered in Rust but **no plugin permission is granted to
  the webview**. Notebook windows get nothing, and `allowed_navigation` pins each to
  `http`, host `127.0.0.1`, and that session's exact port.
- **CSP**: `default-src 'self'`, `script-src 'self'`, `object-src 'none'`, `frame-src
'none'`. `style-src` allows `'unsafe-inline'` because the UI sets a few inline styles.
- **Supply chain**: the ~1.4 GB runtime image is pinned by SHA-256 in
  `trusted-runtimes.json`; `stage-runtime.ps1` refuses an unpinned archive and re-hashes it
  independently before copying.

**Residual risks that are accepted, not overlooked:** the Jupyter token travels in a URL
(that is Jupyter's browser auth mechanism), any local process running as the same user can
reach the loopback port, backups are unencrypted ZIPs, and students execute arbitrary code
by design, that is the product.

---

## 8. Build pipeline

```
scripts/build-runtime.ps1     release step, once per SageMath version. Builds the ~1.4 GB
                              runtime image + checksum manifest. Never run on a student PC.
scripts/stage-runtime.ps1     verifies an image against trusted-runtimes.json and copies it
                              into src-tauri/runtime/   (REQUIRED before any installer build)
npm run tauri build           tsc && vite build, then cargo --release, then WiX
                              -> src-tauri/target/release/bundle/msi/
npm run tauri build -- --no-bundle    exe only; the fast path for a local test build
scripts/runtime/provision.sh  what runs *inside* the image during build-runtime
```

`src-tauri/runtime/` is git-ignored and is a **build input, not output**. `build.rs`
asserts a staged `.tar.xz`/`.tar.gz` is present for release profiles and fails the build
otherwise. Release profile: `lto = true`, `codegen-units = 1`, `panic = "abort"`,
`strip = true`.

Version lives in **three** places that must agree: `package.json`, `src-tauri/Cargo.toml`,
`src-tauri/tauri.conf.json`. (`Cargo.lock` follows automatically.)

---

## 9. The validation gate

Run from the repository root, **sequentially**:

```powershell
npm run format:check
npm run format:rust:check
npm run lint              # Knip: unused files, exports, dependencies
npm run lint:rust         # Clippy with -D warnings
npm run typecheck:tools
npm run build             # tsc && vite build
npm run test:rust
npm run test:ui
```

`npm run format` and `npm run format:rust` apply the shared style rules.

The real integration suite is separate and needs working WSL 2 plus the staged image:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-fresh-install.ps1
```

It imports the pinned image into a randomly named `SageDockQA-<guid>` distribution and
unregisters it afterwards. It can only ever target a throwaway distro: `distro_name()`
asserts the override starts with `SageDockQA-`, and the override compiles only under
`#[cfg(test)]`.

---

## 10. Traps that have actually cost time

Read this section before debugging anything.

- **Never let a screen own a long operation.** This cost a whole release cycle. Setup was
  owned by `Home`: it held the only `setup-progress` subscription, the progress card needed
  a live event _and_ the frontend busy flag, and busy-ness was a string cleared in a
  promise's `finally`. Navigating away lost every event with nothing to recover from, and a
  command that never settled left the app permanently "Setting up SageMath" with no way
  back. **If an operation can outlive a screen, the backend owns it and the screen
  observes it.**
- **A timeout must not be allowed to lie.** The old elevation path killed the outer,
  unelevated PowerShell on timeout, which cannot stop the elevated `dism` it started, and
  then reported the installation as stopped while it was still running. Quiet output is not
  evidence of a hang; check the actual process.
- **`exit $p.ExitCode` from `Start-Process -PassThru` can exit 0 for a run that never
  happened**, because `ExitCode` may be null and `exit $null` is `exit 0`. Anything that
  matters must come back through a result channel that can be validated, not an integer.
- **Do not run two Rust builds concurrently on Windows.** The linker cannot replace a
  running executable (`LNK1104`). Clippy, `cargo test` and `tauri build` must be sequential.
- **Close SageDock before a release build.** A running `sagedock.exe` holds the linker
  lock. Windows Defender (`MsMpEng.exe`) also transiently locks a freshly linked binary,
  producing `Access is denied (os error 5)`, pausing and retrying works.
- **The same lock bites the installer, and there it is easy to misread.** WiX's `light`
  step writes a ~1.4 GB MSI, and Defender or the search indexer touching it mid-write fails
  the bundle with `os error 32` (a sharing violation) _after_ cargo has already succeeded.
  So the exe is genuinely built and updated while `tauri build` reports failure, and an
  MSI may still be sitting there that opens fine and reads back the right ProductVersion
  and file count. That proves the **database**, not the embedded cabinet, which is where a
  truncated write would actually show up. Re-run the bundle instead of shipping it; cargo
  is already up to date, so the retry goes almost straight to WiX. Note the repository
  lives under `C:\Users\…\OneDrive\Desktop`, so the sync client is a third candidate
  whenever it is running.
- **`--offline` Rust commands need a dependency cache** populated by one online build.
- **Clippy is stricter than `cargo test`.** `-D warnings` promotes things like `unused_mut`
  to errors, so a green `cargo test` proves nothing about the gate.
- **`getCurrentWebview()` throws _synchronously_** when `__TAURI_INTERNALS__.metadata` is
  absent, and an uncaught throw inside a `useEffect` unmounts the whole component. Wrap it.
  The UI mock must define `metadata` or the drag-drop subscription is skipped.
- **Never nest `task.run`.** The `lock.current` ref makes the inner call silently no-op.
- **A `<button>` inside a `<button>` is invalid HTML** and Chromium silently breaks it. See
  `NotebookRow.tsx` for the `div` + sibling-buttons pattern, which uses `:hover,
:focus-within` so hover-revealed actions stay keyboard-reachable.
- **Playwright `getByText(..., { exact: true })` matches an element's _full_ text content.**
  Adding an icon glyph or a nested `<span>` to an element silently breaks the match. Icons
  are `aria-hidden` for exactly this reason; keep labels in their own element.
- **Segoe Fluent Icons codepoints are private-use characters.** A wrong one renders as an
  empty box rather than failing. Keep them in the one verified table in `Icon.tsx`, and add
  new ones as escapes, not pasted glyphs.
- **Git Bash mangles POSIX paths passed to `wsl.exe`** (`/bin/sh` becomes
  `C:/Program Files/Git/usr/bin/sh`). Use PowerShell for WSL invocations.
- **`docs/qa/*.png` are tracked on purpose** and regenerated by the UI suite. Open them
  after a UI change: four visual defects once passed the entire test suite and were caught
  only by looking.
- **A WebView2 download decision is synchronous and on the UI thread.**
  `DownloadEvent::Requested` has to set `destination` and return a verdict before it yields,
  so anything asynchronous answers too late, including `tauri-plugin-dialog`'s callback
  pickers, while its `blocking_*` pickers deadlock the event loop when called from there.
  `desktop.rs` uses `rfd`'s synchronous dialog for exactly that reason; do not "tidy" it
  back onto the plugin.
- **`DownloadEvent` is `#[non_exhaustive]`**, so a match on it needs a wildcard arm or it
  will not compile against a future Tauri.
- **Prettier ignores `coding agent onboarding/app.md`** (see `.prettierignore`); it is a
  historical document and reformatting it would produce a meaningless diff.

---

## 11. What is not proven

State these honestly rather than implying coverage that does not exist:

- **The UI tests run in Edge against a mocked IPC bridge.** They prove component logic and
  layout, not that the packaged WebView2 shell or real IPC works.
- **Every native dialog is mocked.** No file or folder picker has been exercised by a test.
  The download Save As dialog and installed-browser detection have both been confirmed
  working by hand, but neither has automated coverage, so a regression in either would be
  silent. Treat "confirmed by the developer" and "tested" as different claims.
- **No clean-Windows verification exists.** Nothing here has been installed on a fresh
  Windows image lacking WSL. Windows-feature installation, UAC, and the reboot/resume path
  are covered by unit tests over pure decision logic only.
- **The elevated install has never been executed.** This is the sharpest instance of the
  point above, and it now covers freshly written code: the elevation supervisor, its
  PowerShell helper, the result file and the process-liveness check were rewritten in the
  setup-reliability work and have only ever run as pure functions in unit tests. On a
  development machine that already has WSL, `install_wsl_elevated` is never reached at all.
  [`../docs/SETUP-RELIABILITY-HANDOFF.md`](../docs/SETUP-RELIABILITY-HANDOFF.md) §7.1 lists
  the six scenarios to run in a VM, in order.
- **Building an installer is not installing one.** MSIs have been verified by reading their
  database back, not by running them.
- **The integration suite is run rarely.** Check
  [`../docs/reviewer/TESTING.md`](../docs/reviewer/TESTING.md) for when it last ran and
  against which version.
- **The installer is not code-signed.** Expect a SmartScreen warning.

---

## 12. Suggested reading order

1. [`src-tauri/src/lib.rs`](../src-tauri/src/lib.rs), the command surface and startup.
2. [`src/lib/commands.ts`](../src/lib/commands.ts), the same surface, typed, from the frontend.
3. [`src-tauri/src/error.rs`](../src-tauri/src/error.rs), the error contract.
4. [`src-tauri/src/runtime/mod.rs`](../src-tauri/src/runtime/mod.rs) then `provision.rs`.
5. [`src-tauri/src/jupyter/mod.rs`](../src-tauri/src/jupyter/mod.rs), the security-critical part.
6. [`src-tauri/src/state.rs`](../src-tauri/src/state.rs), concurrency and the operation lock.
7. [`src-tauri/src/backup.rs`](../src-tauri/src/backup.rs), the largest module, handling user data.
8. [`../docs/reviewer/RESTART-LOGIC.md`](../docs/reviewer/RESTART-LOGIC.md), one worked
   example through a real bug and two rounds of correction. The best single thing to audit.
