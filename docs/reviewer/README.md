# Reviewer's guide to SageDock

Start here. This folder exists so that someone who has never seen this repository can
orient themselves in about twenty minutes and know where to look for anything else.

**What SageDock is:** a Windows desktop application that makes SageMath, Python and
JupyterLab usable by a student with no technical knowledge and no terminal. It installs and
manages its own WSL 2 Linux distribution containing SageMath, while keeping the student's
notebooks in ordinary Windows folders outside that distribution.

**Why it is being reviewed:** to decide whether it can be installed on shared university
lab computers.

**Current version:** 1.4.6. Rust and frontend are 14,814 and 6,816 lines respectively.

> The prose below was written at 1.3.1 and its narrative still holds, but treat version
> numbers and counts in this folder as of that date unless a document says otherwise.
> [TESTING.md](TESTING.md) carries the counts that were actually re-measured.

## Read these in this order

| Document                               | What it answers                                                                                                                          |
| -------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| [ARCHITECTURE.md](ARCHITECTURE.md)     | How the code is laid out and which file owns which concern. Includes a suggested reading order for the source itself.                    |
| [SECURITY.md](SECURITY.md)             | The security posture, claim by claim, with the file implementing each one, and the gaps. **Most relevant to a lab deployment decision.** |
| [LAB-DEPLOYMENT.md](LAB-DEPLOYMENT.md) | What the installer does, what it needs from the base image, what it writes where, and what is unverified.                                |
| [TESTING.md](TESTING.md)               | The real test counts, how the suites are structured, and what they do not establish.                                                     |
| [RESTART-LOGIC.md](RESTART-LOGIC.md)   | One worked example, followed through a bug and two rounds of correction. The best single piece of code to audit.                         |

## Three things to know before you start

**1. The repository has no git history.** `git log` fails: there are zero commits, and
everything is untracked on branch `master`. You cannot review this by reading diffs. The
narrative history lives in the documents listed below instead.

**2. The specification uses a different product name.**
[`app.md`](../../coding%20agent%20onboarding/app.md) is the original 1,345-line product and
infrastructure specification, and it calls the product **DataLab**. The name was later
changed to SageDock, and all code, identifiers and paths use SageDock. `app.md` remains the
source of truth for intent, but some of it has been deliberately superseded (see the
currency table). It now lives in
[`coding agent onboarding/`](../../coding%20agent%20onboarding/README.md) alongside the
previous handoff documents and a detailed repository guide.

**3. Documentation currency varies, and some of it is stale.** This is the most likely way
to get lost, so the table is explicit.

| Document                                                                                                           | Status                                | Notes                                                                                                                                                                                                       |
| ------------------------------------------------------------------------------------------------------------------ | ------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`coding agent onboarding/REPO_CONTEXT.md`](../../coding%20agent%20onboarding/REPO_CONTEXT.md)                     | **Current, newest**                   | Detailed repository guide written at 1.4.3: architecture, invariants, the gate, and the traps. The best single document for someone changing the code rather than judging it.                               |
| [`docs/reviewer/`](.)                                                                                              | **Current**                           | This folder, written at 1.3.1.                                                                                                                                                                              |
| [`docs/DESIGN.md`](../DESIGN.md)                                                                                   | **Current, authoritative**            | The design system. Binding for any UI change.                                                                                                                                                               |
| [`docs/BACKUP-FORMAT.md`](../BACKUP-FORMAT.md)                                                                     | **Current, authoritative**            | Backup container and compatibility rules.                                                                                                                                                                   |
| [`coding agent onboarding/TOOLS-LAUNCHER-HANDOFF.md`](../../coding%20agent%20onboarding/TOOLS-LAUNCHER-HANDOFF.md) | **Current, newest**                   | Compilers and build tools, Home's capability strip, the JupyterLab launcher, and the tool icons. States plainly which parts are unit-tested and which have never been executed.                             |
| [`coding agent onboarding/HOME-LAUNCH-AUDIT.md`](../../coding%20agent%20onboarding/HOME-LAUNCH-AUDIT.md)           | **Current, partly superseded**        | An audit of the preceding work. Supersedes the "deferred checks" section of CODE-QUALITY.md. Its description of the Home launcher was replaced by TOOLS-LAUNCHER-HANDOFF.md, and it is marked in place.     |
| [`coding agent onboarding/CODE-QUALITY.md`](../../coding%20agent%20onboarding/CODE-QUALITY.md)                     | **Mostly current, one stale section** | Its "Checks run and checks deferred" list was written under an instruction not to compile. Everything it lists as deferred has since been run. Read HOME-LAUNCH-AUDIT.md and TESTING.md for the real state. |
| [`coding agent onboarding/QA-REVIEW.md`](../../coding%20agent%20onboarding/QA-REVIEW.md)                           | **Historical**                        | A dated record of earlier fixes. Defers to CODE-QUALITY.md for current results.                                                                                                                             |
| [`coding agent onboarding/IMPLEMENTATION.md`](../../coding%20agent%20onboarding/IMPLEMENTATION.md)                 | **Historical**                        | Implementation notes from earlier milestones.                                                                                                                                                               |
| [`app.md`](../../coding%20agent%20onboarding/app.md)                                                               | **Intent, partly superseded**         | §4.3's folder template and §6's primary action were deliberately replaced by the Home JupyterLab launcher; see HOME-LAUNCH-AUDIT.md.                                                                        |
| [`docs/qa/*.png`](../qa/)                                                                                          | **Current**                           | Screenshot evidence, regenerated by the UI suite.                                                                                                                                                           |

## Build and verify it yourself

Requirements: Node.js, Rust (MSVC toolchain), Tauri prerequisites, and installed Microsoft
Edge for the UI tests.

```powershell
npm install
npm run tauri dev      # run the full app
```

The full validation gate, and the results it produced at 1.3.1, are in
[TESTING.md](TESTING.md). Two warnings that will otherwise waste your time:

- **Do not run two Rust builds concurrently on Windows** (`LNK1104`). Clippy, `cargo test`
  and `tauri build` must be sequential.
- **Building an installer requires a ~1.4 GB runtime image staged first** into
  `src-tauri/runtime/` via `scripts/stage-runtime.ps1`. That directory is git-ignored and is
  a build _input_. Without it, `npm run tauri build` fails.

## A review checklist

Ordered by how much damage a defect would do.

- [ ] **Notebook server exposure.** Confirm `--ServerApp.ip=127.0.0.1` and
      `allow_remote_access=False` in `jupyter/mod.rs`, and that no code path can widen them.
      This is the one that matters on a lab network.
- [ ] **Shell injection.** Confirm no user-supplied value is concatenated into a command
      string. Everything should be `argv` arrays through `wsl --exec`.
- [ ] **Path handling from the webview.** Confirm no IPC command accepts an absolute path,
      and that native pickers run in Rust. `src/lib/commands.ts` is the whole surface.
- [ ] **Capability grants.** `capabilities/default.json` should grant `core:default` only,
      and notebook windows nothing.
- [ ] **Deletion safety.** `forget_workspace` must not delete files. Check every
      `remove_dir_all` and `unregister_distro` call site for what it can reach.
- [ ] **Elevation.** Confirm the app does not run elevated and that elevation is confined to
      Windows-feature installation.
- [ ] **WSL blast radius.** Confirm `wsl --shutdown` appears nowhere and only SageDock's own
      distribution is terminated or unregistered.
- [ ] **Runtime provenance.** Decide whether you trust the SHA-256 pinned in
      `trusted-runtimes.json`, because that hash is the only thing establishing what is
      inside the 1.4 GB image you would be deploying.
- [ ] **Error honesty.** Spot-check that `user_files_safe` is accurate on failure paths; it
      defaults to `true`.
- [ ] **Open the screenshots** in [`docs/qa/`](../qa/) rather than trusting the test count.
      Four visual defects once passed the entire suite.

## Known limitations, stated plainly

These are the things a reviewer should not have to discover for themselves.

- **The installer is not code-signed.** Expect a SmartScreen warning.
- **It has never been installed on a clean Windows image.** Windows-feature installation,
  UAC, and the reboot/resume path are covered by unit tests over pure logic only.
- **The UI tests run in Edge against a mocked IPC bridge**, so they do not prove the real
  WebView2 shell or any native dialog works.
- **The five real integration tests were last run at 1.2.0**, not at 1.3.1.
- **Backups are unencrypted ZIPs** and copy workspace files as-is, deliberately.
- **Students run arbitrary code by design.** That is the product.
