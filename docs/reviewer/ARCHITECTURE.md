# Architecture and code map

Written for a reviewer who needs to find things quickly in a repository they have not seen
before. The goal is that after reading this you can predict which file any given concern
lives in.

## Shape of the product

SageDock is a **Tauri v2** desktop application: a Rust binary that owns a WebView2 window
rendering a React 19 / TypeScript frontend built by Vite. It exists to make SageMath,
Python and JupyterLab usable without a terminal.

The interesting design decision, and the one most of the code follows from, is that
SageDock **installs and owns its own WSL 2 Linux distribution** containing SageMath, while
the student's notebooks stay in ordinary Windows folders under `Documents\SageDock`,
outside that distribution. The runtime is disposable infrastructure; the notebooks are not.
Repair and reinstall replace the former and must never touch the latter.

Rough size: **13,030 lines of Rust** across 35 files, **4,135 lines of frontend** across 22.
(Measured at 1.4.3; the prose in this folder was otherwise written at 1.3.1.)

## Process and window model

```
sagedock.exe  (Rust, src-tauri/src/main.rs -> lib.rs::run)
├── window "main"           the React UI, capability: core:default
├── window "notebook-<port>"  JupyterLab, external URL, no IPC capabilities
└── child process: wsl.exe --exec ... sagedock-jupyter   (one per open workspace)
```

[`src-tauri/src/lib.rs`](../../src-tauri/src/lib.rs) (214 lines) is the entry point and the
best first file to read. It registers plugins, resolves every path once into `AppPaths`,
installs the window-close and exit guards and the drag-drop handler, and lists all 48 IPC
commands. Note the ordering comment on `single_instance`: it must be registered first, and
its job is to ensure exactly one SageDock owns the environment.

`main.rs` is 6 lines and simply calls `run()`.

## The frontend/backend boundary

**All 48 IPC commands** are registered in one `generate_handler!` block in `lib.rs`, split
across three modules:

| Module                                           | Count | Concern                                                                    |
| ------------------------------------------------ | ----- | -------------------------------------------------------------------------- |
| [`commands.rs`](../../src-tauri/src/commands.rs) | 13    | app info, config, browsers, onboarding, setup, notebook creation, shutdown |
| [`desktop.rs`](../../src-tauri/src/desktop.rs)   | 18    | notebook opening, the launcher, downloads, tools, recovery, diagnostics    |
| [`home.rs`](../../src-tauri/src/home.rs)         | 17    | environment status, workspaces, imports, backups                           |

Note the pair `open_notebook` and `open_jupyter_home`. They are separate commands because
they open different screens: the first addresses a file or folder inside a session root, the
second opens JupyterLab's own landing page rooted at the default SageDock folder. See
[TOOLS-LAUNCHER-HANDOFF.md](../../coding%20agent%20onboarding/TOOLS-LAUNCHER-HANDOFF.md).

On the frontend, [`src/lib/commands.ts`](../../src/lib/commands.ts) is the **only** file
that calls `invoke`. Components import the typed `commands` object from it. That is a
deliberate auditing property: the complete set of operations the webview can trigger is one
readable list, and no command accepts an absolute path or shell text. The TypeScript
interfaces there mirror the Rust `serde` shapes, and the comments name the Rust file each
one mirrors.

Events stream from Rust to the UI rather than being polled: `setup-progress`,
`backup-progress` and `tool-progress`, plus `close-requested` for the window-close guard
and `files-dropped` reporting what a drag from Windows onto the window copied in. The last
one is an event rather than a return value because a drop has no caller — and because the
copy happens in Rust, so the dropped absolute paths never reach the frontend.

## Rust module layering

The layering is intentional and stated in
[`runtime/mod.rs`](../../src-tauri/src/runtime/mod.rs). Violating it is the main thing to
watch for in review.

### `runtime/` — owns installation and the Linux environment

| File           | Lines | Responsibility                                                                                                          |
| -------------- | ----- | ----------------------------------------------------------------------------------------------------------------------- |
| `wsl.rs`       | 590   | **The only module that executes WSL commands.** Distro naming, import, export, terminate, unregister, path translation. |
| `image.rs`     | 567   | Finds and verifies the SageMath package against `trusted-runtimes.json`.                                                |
| `workspace.rs` | 274   | Prepares Windows folders; resolves Documents through a fallback chain.                                                  |
| `provision.rs` | 763   | The setup state machine, composing the above. Owns `PersistedSetupState`.                                               |
| `health.rs`    | 89    | `preflight` (launch checks) and `fast` (is the environment usable).                                                     |
| `repair.rs`    | 101   | Replace or rebuild the environment, preserving notebooks.                                                               |

`runtime::contract` pins the four paths every runtime image must provide
(`/opt/sagedock/runtime.json`, and the `sagedock-jupyter`, `sagedock-verify`,
`sagedock-selftest` binaries). The app depends only on these, never on how the image was
assembled inside, so a new image can change SageMath versions without an app update.

### `system/` — read-only diagnostics

Nine modules, each a single check, all **read-only**; installation and repair belong to
`runtime/`. Every check follows the gather/interpret split described in
[TESTING.md](TESTING.md). `types.rs` holds `CheckItem`, `HealthState` and the
`compute_overall` rollup. `run_system_check(setup_awaiting_restart: bool)` runs all eight
checks and rolls them up.

`system/reboot.rs` is worth reading in full even though it is small: its module comment
documents a real bug where three unrelated Windows "restart pending" registry indicators
were treated as one boolean, which made SageDock report a required restart on nearly every
machine. See [RESTART-LOGIC.md](RESTART-LOGIC.md).

`system/process.rs` is shared with `runtime/`, so console-window suppression and output
decoding behave identically everywhere.

### `jupyter/` — the notebook service

`mod.rs` (435 lines) owns the server lifecycle: loopback binding, per-session CSPRNG token,
OS-assigned port, readiness by polling `/api/status` rather than sleeping, and graceful
shutdown that asks Jupyter to stop before killing `wsl.exe`. `notebook.rs` handles `.ipynb`
creation and validation. Security details are in [SECURITY.md](SECURITY.md).

### Application layer

| File            | Lines | Responsibility                                                                                    |
| --------------- | ----- | ------------------------------------------------------------------------------------------------- |
| `backup.rs`     | 1737  | Portable ZIP backup and restore. The largest module; see [BACKUP-FORMAT.md](../BACKUP-FORMAT.md). |
| `scientific.rs` | 1145  | Compilers and build tools: an allowlisted catalogue, verified by compiling and running programs.  |
| `workspaces.rs` | 863   | The workspace registry: folders the student has added.                                            |
| `library.rs`    | 601   | Notebook discovery, import, Downloads scanning, relative-path resolution, Recycle Bin deletion.   |
| `state.rs`      | 537   | `AppState`, the operation lock, running servers, the config mutex.                                |
| `config.rs`     | 233   | Settings persistence, including the onboarding flag and the chosen browser.                       |
| `browsers.rs`   | 216   | Installed-browser detection from `SOFTWARE\Clients\StartMenuInternet`, for the first-run choice.  |
| `error.rs`      | 93    | `AppError` / `ErrorSeverity`.                                                                     |
| `logging.rs`    | 50    | Structured logging.                                                                               |
| `storage.rs`    | 43    | `atomic_write`.                                                                                   |

## Cross-cutting patterns

**Every failure is an `AppError`.** [`error.rs`](../../src-tauri/src/error.rs) defines a
struct that answers four questions the product spec requires of any user-facing failure:
what happened (`title`, `message`), is my work safe (`user_files_safe`), can SageDock fix it
(`recovery_actions`), and what is the technical detail (`technical_details`, shown only
behind a disclosure). Commands must never return raw process output or bare strings. When
reviewing a new error path, check that `user_files_safe` is accurate — it defaults to
`true`.

**One long operation at a time.** `AppState::begin_operation` returns an `OperationGuard`
whose `Drop` clears the label, so an early return or panic cannot leave the app permanently
busy. The label is a human sentence fragment completing "SageDock is still …", because a
bare boolean once let the UI claim it was installing SageMath while actually writing a
backup.

**One Jupyter server per workspace root**, capped at `MAX_LIVE_SERVERS = 6`. A Jupyter
server exposes exactly one directory tree, so re-rooting a shared server would either kill
a running calculation or show the wrong folder.

**Atomic writes for app-owned JSON.** `storage::atomic_write` writes to a randomly named
temp file, `sync_all`s, then renames. Never used for notebooks, which Jupyter owns.

**Paths resolved once.** `AppPaths` is built in `lib.rs::run` and everything reads from it.
Note the Documents resolution chain: on a machine with OneDrive-redirected Documents the
known-folder API can return an empty path, which would drop notebooks into AppData where
nobody would find them.

**Logging.** JSON lines to a daily-rotating file in the Tauri app log directory. Each call
sets an explicit `target` naming a subsystem (`installer`, `launcher`, `wsl`, `runtime`,
`jupyter`, `repair`, `config`, …). Verbose logging is opt-in via `SAGEDOCK_LOG=debug`, never
on by default.

## Frontend structure

```
src/main.tsx            mounts App
src/App.tsx             ConfigProvider > TaskProvider > Root; Root renders EITHER the
                        first-run introduction OR HashRouter + AppShell (7 routes)
src/components/
  AppShell.tsx          nav + content, task bar, error banner, notice, close dialog
  NavRail.tsx           7-entry navigation pane, collapses to icons below 900px
  Icon.tsx              Segoe Fluent Icons codepoint table (one verified place)
  WorkspaceCard.tsx  NotebookRow.tsx  ChoiceDialog.tsx  ConfirmDialog.tsx  ErrorBanner.tsx
src/pages/              Home (1292), Tools (419), Onboarding (219), Settings, Recovery,
                        Diagnostics, Help, About
src/state/
  ConfigContext.tsx     theme, browser and onboarding settings; tolerates a failed load
  TaskContext.tsx       the busy/error/message surface used by every page
src/styles/
  theme.css   (261)     tokens, type ramp, base elements
  layout.css  (1437)    structure and components
```

`HashRouter` rather than `BrowserRouter` because the app is served from a file origin in
the packaged shell. Routing is `/`, `/tools`, `/recovery`, `/diagnostics`, `/settings`,
`/help`, `/about`.

The router is not reached at all until `onboarding_complete` is true: `Root` renders the
first-run introduction **instead of** the whole shell. Anything that gates the app this way
has to be mirrored in the UI test fixture — the mock reports the introduction as already
seen by default, and forgetting that would fail every UI test at once.

Styling rule worth enforcing in review: components carry **class names only**. Colours and
spacing come from tokens in `theme.css`; there are no page-specific stylesheets. The full
rationale, including the two-accent-token decision and why filled controls are deliberately
not transitioned, is in [DESIGN.md](../DESIGN.md).

## Build pipeline

```
scripts/build-runtime.ps1     release step, once per SageMath version, builds the
                              ~1.4 GB runtime image + checksum manifest (never run
                              on a student machine)
scripts/stage-runtime.ps1     verifies an image against trusted-runtimes.json and
                              copies it into src-tauri/runtime/  (required before
                              any installer build)
npm run tauri build           tsc && vite build, then cargo --release, then WiX
                              -> src-tauri/target/release/bundle/msi/
scripts/runtime/provision.sh  what runs *inside* the image during build-runtime
```

`src-tauri/runtime/` is git-ignored and is a **build input**, not output. An installer build
fails without it. The release profile uses `lto = true`, `codegen-units = 1`,
`panic = "abort"` and `strip = true`.

## Suggested reading order

1. `src-tauri/src/lib.rs` — the whole command surface and startup sequence.
2. `src/lib/commands.ts` — the same surface from the frontend, with types.
3. `src-tauri/src/error.rs` — the error contract every path funnels through.
4. `src-tauri/src/runtime/mod.rs` then `provision.rs` — the setup state machine.
5. `src-tauri/src/jupyter/mod.rs` — the security-critical part.
6. `src-tauri/src/state.rs` — concurrency and the operation lock.
7. `src-tauri/src/backup.rs` — the largest module, and the one that handles user data.
