# SageDock

A Windows desktop app that makes SageMath, Python, and JupyterLab usable without a
terminal. SageDock installs and manages its own Linux runtime, keeps your notebooks in
ordinary Windows folders, and handles setup, repair, and recovery on your behalf.

Built with Tauri (Rust) + React/TypeScript.

## Developer

SageDock is developed by **Yassin Eisa**.

## License

SageDock's original code is released under the MIT License — see [`LICENSE`](LICENSE).

> Copyright (c) 2026 Yassin Eisa

The MIT license covers SageDock's own source code only. The bundled SageMath runtime and
every other third-party component retain their own licenses and copyright holders; see
[`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md).

## Where your work lives

Notebooks are stored in ordinary Windows folders — by default under
`Documents\SageDock` — deliberately _outside_ the Linux runtime. The runtime is treated as
disposable infrastructure that repair and reinstall may replace at any time; your files are
not. They stay visible in File Explorer and get picked up by whatever backup tool you
already use.

You can keep several **workspaces** (for example `Calculus`, `Physics`, `Statistics`).
Each one is a plain folder you can move, copy, or back up yourself.

**Open JupyterLab** is the main Home action. It opens JupyterLab's own landing page
directly, without creating a notebook or asking you to choose a workspace. It is always
rooted at `Documents\SageDock` rather than following whichever course you opened last, so
it stays a general "start working" action. Workspace cards remain the way to open a
specific course folder.

New workspaces are empty folders. New and imported notebooks are saved directly inside
the folder; you can organize files and subfolders yourself. Existing folders from earlier
versions are preserved, and notebooks in those folders remain accessible.

## Backups

**Create backup** on the Home screen writes a portable `.zip` containing your workspaces,
their notebooks, datasets, and checkpoints, plus a versioned manifest with integrity
checksums. A backup is restorable by a later SageDock installation on a different computer
and does not depend on your Windows username, absolute paths, app settings, or an existing
WSL installation.

SageDock does not export its session tokens or machine settings, and known runtime/cache
folders are excluded. Workspace files are copied as-is: a notebook, `.env`, or Git
configuration can contain personal information or credentials. Backups are unencrypted ZIPs.

Backup and restore are available before SageMath setup. Restore verifies the archive and
adds separate workspace folders without overwriting existing coursework.

This is distinct from **Recovery → Back up & reinstall**, which snapshots the _Linux
runtime_ and is not portable between machines.

## Development

Requirements: Node.js, Rust (MSVC toolchain), and the Tauri prerequisites.

```sh
npm install
npm run dev              # frontend only
npm run tauri dev        # full app
npm run build            # typecheck + build frontend
npm run typecheck:tools  # typecheck Vite, Playwright, and UI tests
npm run format:check     # frontend, configuration, and documentation formatting
npm run format:rust:check
npm run lint             # unused frontend files, exports, and dependencies
npm run lint:rust        # strict Clippy checks
npm run test:ui          # mocked UI tests (Playwright)
npm run test:rust        # Rust unit tests; integration tests run separately below
```

Building an installer requires the runtime image to be staged first:

```sh
powershell -File scripts/stage-runtime.ps1
npm run tauri build
```

`powershell -File scripts/test-fresh-install.ps1` runs all six integration tests against
a throwaway `SageDockQA-*` distribution, then removes that QA registration after checking
its install path. It requires working WSL 2 and the staged runtime image. Test artifacts
remain under `test-results/`. It never targets a personal SageDock environment.

Use `npm run format` and `npm run format:rust` to apply the shared style rules. Offline
Rust checks require the dependency cache to have been populated by an initial build.

## Documentation

**Reviewing this repository for the first time?** Start with
[`docs/reviewer/`](docs/reviewer/README.md). It is written for exactly that: an orientation
guide, a code map, the security posture claim by claim, lab-deployment notes, and an honest
account of what the tests do and do not establish.

**Picking this up as a coding agent, or as a new contributor?** Start with
[`coding agent onboarding/`](coding%20agent%20onboarding/README.md), and in particular
[REPO_CONTEXT.md](coding%20agent%20onboarding/REPO_CONTEXT.md) — a detailed guide to the
architecture, dependencies, core logic, invariants, and validation gate. That folder also
collects the product specification and every previous handoff document.

The rest of [`docs/`](docs/) holds the two current, binding specifications: the
[design system](docs/DESIGN.md) and the
[backup format and compatibility rules](docs/BACKUP-FORMAT.md).

## Recommended IDE setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
