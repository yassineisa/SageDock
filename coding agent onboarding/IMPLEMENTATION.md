# Implementation notes — environment shutdown, backups, workspaces, licensing

> Historical record of the initial implementation. The subsequent review found and fixed
> safety and lifecycle problems described here. Read [QA-REVIEW.md](QA-REVIEW.md) for the
> current behavior, measured results, remaining limitations, and continuation instructions.

Covers the four features added on top of SageDock 1.1.0. Written for whoever picks this up
next. The backup container is specified separately in [`BACKUP-FORMAT.md`](../docs/BACKUP-FORMAT.md).

## 1. Stopping the computing environment

**Where:** [`home.rs`](../src-tauri/src/home.rs) (`environment_status`, `stop_environment`),
[`wsl.rs`](../src-tauri/src/runtime/wsl.rs) (`distro_is_running`), `Home.tsx`.

The Home screen shows _Computing environment running_ / _stopped_ with a **Stop SageMath**
button, and explains that stopping frees memory and ends running calculations. Stopping
confirms first, warning that unsaved notebook changes may be lost.

Three decisions worth keeping:

**Status is observed, not remembered.** `distro_is_running()` runs
`wsl.exe --list --running --quiet`. `--quiet` prints bare distribution names — no header, no
localized prose — which is why comparing it does not violate the module's rule against
parsing `wsl.exe` output. Exit code alone cannot answer the question: it is non-zero both
when nothing is running and when something failed.

**Only SageDock's own distribution is stopped.** Every stop path is
`wsl --terminate <name>`. `wsl --shutdown` is never called anywhere — it would stop every
distribution on the machine, including someone's unrelated work. Verified by search: the
string `--shutdown` appears in exactly two places in the repository, both comments
explaining why it is not used.

**Success is not assumed from a return code.** After terminating, `stop_environment`
re-observes; if the environment is still running it returns `STOP_DID_NOT_TAKE_EFFECT`
rather than reporting success.

The app stays open afterwards. `state.session_for(root)` starts a server on demand, so the
next notebook or workspace launch brings the environment back automatically.

## 2. Folder-based workspaces

**Where:** [`workspaces.rs`](../src-tauri/src/workspaces.rs), `home.rs`, `state.rs`,
`WorkspaceCard.tsx`.

A workspace is an ordinary Windows folder. SageDock records where it is and when it was last
opened, and nothing else. New ones are created under
`Documents\SageDock Workspaces\<Name>` — a **sibling** of the default `Documents\SageDock`,
not a child. Nesting would make every workspace appear inside the default workspace's file
browser and be counted twice in a backup.

The list lives in `workspaces.json` in the app data directory, written atomically.
`update_workspaces` rolls back the in-memory list if the save fails, so what is on screen
always matches what will be there next launch.

**Name validation** rejects what Windows cannot store faithfully: `<>:"/\|?*`, control
characters, trailing dot or space (Windows silently strips these, which would desynchronise
the recorded path from the real folder), reserved device names in any casing or with any
extension (`CON`, `con.txt`, `LPT9`), and names over 64 characters.

**Rename moves the folder on disk**, so the card and File Explorer agree. It refuses if the
destination exists, and the `id` survives so the active selection is not lost.

**Remove is a list operation, never a delete.** `forget` removes the entry and touches
nothing on disk; the confirmation says so explicitly. The last workspace cannot be removed.

**Missing, moved, or inaccessible folders** are classified per-read as `available`,
`missing`, or `unreadable` — three states rather than a boolean, because "the folder is
gone" and "the folder is there but unreadable" need different advice. The card states the
problem in words and disables the actions that would fail. `active_workspace_dir()` falls
back to the default workspace if the active one has vanished.

**Switching workspaces does not stop calculations.** A Jupyter server exposes exactly one
directory tree, so re-rooting a shared server would either kill the first workspace's
kernels or show the wrong folder. Instead `RunningServer` gained a `root` and `AppState`
holds a small set keyed by it; returning to a workspace rejoins its existing session.
Capped at `MAX_LIVE_SERVERS = 6`.

## 3. Portable backup and restore

**Where:** [`backup.rs`](../src-tauri/src/backup.rs), `home.rs`, `Home.tsx`. Format details
in [`BACKUP-FORMAT.md`](../docs/BACKUP-FORMAT.md).

Create and Restore sit on Home under _Backups_. Both use native pickers opened by the
backend — the webview never supplies a filesystem path. The chosen backup is held in
`AppState.selected_backup` between previewing and restoring, so restore needs no path from
the frontend.

Restore previews first: what is inside, how big, whether it is compatible, and the exact
name each workspace will land under. Nothing is changed until the user confirms.

## 4. Licensing and credit

`LICENSE` (MIT, © 2026 Yassin Eisa), `THIRD-PARTY-NOTICES.md`, README credit, and a visible
**About** section at the bottom of _Guides & help_. All four state the same boundary: MIT
covers SageDock's own code; the bundled SageMath runtime and every other third-party
component keep their own licenses. Pre-existing notices (`src-tauri/runtime/README.txt`) are
untouched.

## Defects fixed along the way

Three problems from the earlier code review were in the path of this work and are now fixed:

- **`new_notebook` was half-gutted** by a mechanical edit script — it started a Jupyter
  server before creating the file, and contained an unreachable environment check. It now
  health-checks, creates the file, and returns; _opening_ starts the server, which removes a
  duplicated server start per notebook.
- **The setup mutex was doing duty as a global busy flag**, so creating a backup told the
  user SageDock was installing SageMath. Replaced with a labelled operation slot
  (`begin_operation(operation::BACKUP)`); every refusal now names what is actually running.
- **`get_jupyter_status` blocked the main thread** on a 2-second HTTP call. Now `async`.

## Visual redesign

Outside this document's scope, and recorded elsewhere. The interface was rebuilt twice
after this document was written: first on an editorial system in 1.1.1, then — replacing it
— on **Metro principles with Windows 11 / Fluent conventions** in 1.1.3, which is what ships
today.

[`DESIGN.md`](../docs/DESIGN.md) specifies the current system only; [`QA-REVIEW.md`](QA-REVIEW.md)
records both changes and supersedes any earlier description. Both were presentation-only:
`theme.css` and `layout.css` were rewritten and component markup restyled, but no command,
serialized shape, or safety rule changed.

## Changed files

The repository has no commits, so `git status` cannot produce a diff. Enumerated by hand:

**New:** `src-tauri/src/workspaces.rs`, `src-tauri/src/backup.rs`, `src-tauri/src/home.rs`,
`src/components/WorkspaceCard.tsx`, `LICENSE`, `THIRD-PARTY-NOTICES.md`,
`docs/IMPLEMENTATION.md`, `docs/BACKUP-FORMAT.md`, `docs/DESIGN.md`.

**Modified:** `src-tauri/Cargo.toml` (added `zip 2`, deflate only),
`src-tauri/src/state.rs` (operation labels, per-workspace servers, workspace store),
`src-tauri/src/commands.rs`, `src-tauri/src/desktop.rs`, `src-tauri/src/lib.rs`,
`src-tauri/src/runtime/wsl.rs` (`distro_is_running`), `src-tauri/src/jupyter/mod.rs`
(`RunningServer.root`), `src/lib/commands.ts`, `src/pages/Home.tsx`, `src/pages/Help.tsx`,
`tests/ui/workspace.spec.ts`, `README.md`, and — rewritten wholesale for the redesign —
`src/styles/theme.css` and `src/styles/layout.css`.

Untouched by design: the logo, setup/provisioning, recovery, diagnostics, the backup format,
and the runtime image contract. The redesign changed presentation only; no command,
serialized shape, or safety rule was altered by it.

## Test results

Measured on 2026-09-17, on the development machine.

| Suite                          | Result                                                                |
| ------------------------------ | --------------------------------------------------------------------- |
| `cargo test`                   | **158 passed, 0 failed, 5 ignored** (163 total; 109 before this work) |
| `cargo check --all-targets`    | clean — no errors, no warnings                                        |
| `npm run build` (`tsc` + vite) | clean                                                                 |
| `npm run test:ui` (Playwright) | **9 passed**                                                          |

Counts are as measured, not as authored: some of the growth beyond the features described
here came from edits made directly on disk (a manifest size cap in `backup.rs`, failing
loudly rather than truncating when a workspace exceeds the walk limits, `Promise.allSettled`
in `Home.tsx`, and an extra pre-setup UI test). Those were kept, not reverted.

New coverage includes Windows name rules and reserved devices; forget-keeps-every-file;
rename carries files and refuses collisions; corrupt and newer-version workspace lists;
active-workspace fallback; operation labelling; the backup round trip (real ZIP written,
verified, restored alongside the original); and four archive-safety tests built byte-by-byte
so they cannot pass vacuously — wrong checksum, entry outside its workspace, missing entry,
and a backup from a newer SageDock.

**Mocked versus real.** The Playwright tests stub every IPC call. They prove the screens
render, wire up, confirm destructive actions, and stay keyboard-reachable in light and dark.
They prove **nothing** about WSL, SageMath, real backups, or real folders. The Rust tests are
unit-level against temporary directories. The real stack is exercised only by
`scripts/test-fresh-install.ps1`. It had not been run when this section was written; it has
since passed — see [`QA-REVIEW.md`](QA-REVIEW.md) for that run and what it actually covered.

## Limitations and what is not verified

**Superseded — do not rely on the paragraph this replaces.** When this was written, nothing
had been exercised on real hardware. That is no longer true: `scripts/test-fresh-install.ps1`
has since passed against a real disposable distribution, covering stop, relaunch, two
concurrent workspace servers, and both kernels executing again after restart.

[`QA-REVIEW.md`](QA-REVIEW.md) is authoritative for what that run proved and what genuinely
remains unverified — which still includes real backup/restore of a large workspace, restoring
into a fresh configuration on a second machine, and **clean-Windows compatibility, which is
untested and must not be claimed**.

Known limitations in what was built:

- **Rename is conservatively blocked** whenever _any_ notebook server is alive, not only one
  rooted in the workspace being renamed. Correct but coarser than necessary.
- **Backups are always full copies** — no incremental or deduplicated mode. A 10 GB
  workspace produces a roughly 10 GB file.
- **Change detection during backup is size + mtime.** A modification within the same second
  that leaves the size identical could slip through.
- **A seventh concurrent workspace is refused** rather than evicting an existing server,
  since evicting would kill running calculations.
- **Restore requires a drive-letter path.** `add_existing` validates through
  `windows_path_to_wsl`, so restoring onto a UNC or network location is refused.
- **`environment_status` spawns `wsl.exe` on every Home refresh**, a small cost per poll.

Still open from the earlier review, untouched here:

- `runtime/repair.rs` writes a multi-GB runtime backup on every rebuild and never removes
  old ones, so repeated repairs fill the disk.
- `provision.rs` clears the persisted `completed` flag _before_ preflight runs, so a
  transient preflight failure downgrades a working installation to "needs setup".

## Installer and continuation

Both live in [`QA-REVIEW.md`](QA-REVIEW.md), which supersedes this document for packaging,
measured results, and next steps. In short: a verified 1.1.1 MSI has been built but not
installed anywhere, and the repository still has **zero commits** — everything described
here exists only as working files on one machine.

When bumping versions, change `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`,
`package.json`, **and `package-lock.json`** together. Miss the lockfile and npm metadata
disagrees with the shipped package.
