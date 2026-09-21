# Home launcher, plain folders, and follow-up audit

This pass was implemented and tested against **1.2.0**, as a local test-build update.
On resuming on 18 September, the shared repository had independently changed to **1.3.1**
and its installer was at `SageDock-1.3.1-Installer.msi`. Those subsequent changes were not
made by this pass and that replacement installer has not been verified by this audit.

## Product changes

- Home now leads with a large **Open JupyterLab** button. It opens the existing JupyterLab
  interface without creating a notebook or requiring a workspace selection. The initial
  folder is the ordinary SageDock folder in Documents; after launching a course folder,
  it reopens that current folder. The existing browser/embedded-view preference is honored.

  > **Superseded in two ways** by the later tools-and-launcher pass; see
  > [TOOLS-LAUNCHER-HANDOFF.md](TOOLS-LAUNCHER-HANDOFF.md). The button no longer follows the
  > last-launched course folder — it is always rooted at `Documents\SageDock` — and it now
  > opens JupyterLab's `/lab` landing route rather than `/lab/tree/`, which is the file
  > browser. It also uses a dedicated `open_jupyter_home` command instead of
  > `open_notebook("")`.

- The button uses the existing `open_notebook` command with an empty path. Its operation
  guard, runtime health check, authenticated readiness check, kernel verification, and
  reuse of an existing server remain in effect. `prepare_notebook_session` exposes the
  shared backend path to the real integration test without requiring a desktop window.
- New workspace folders are empty. Setup, health checks, repair, and workspace launch
  create only the root directory. Creating or importing a notebook saves it at that root.
  Name collisions still choose another filename; imports still preserve the original.
- Existing folders are neither moved nor deleted. The recursive recent-notebook browser,
  notebook path validation, and backup/restore still support old or user-created subfolders.
  Jupyter may create its own checkpoint folder when saving; this is notebook infrastructure,
  not a SageDock directory template.
- Workspace and notebook creation buttons are secondary to the direct launcher. Help and
  README describe the new behavior.

This intentionally supersedes the suggested folder template in `app.md` section 4.3 and
the example primary action in section 6, following the user's newer instruction. Windows
storage and the SageMath/Jupyter architecture remain unchanged.

## Audit of the previous agent's changes

### Attribution, About, and logo

Confirmed the footer component and styles are removed, About is a separate accessible
navigation entry, and attribution remains there. The SVG logo is included by Vite and
the recoloured installer icon assets remain declared in Tauri configuration. The rebuilt
screenshots show the burgundy logo and the footer-free layout. Home was inspected in light
theme and at the minimum width in dark theme; About was inspected in dark theme.

### Restart warning corrections

The previous fix correctly separated third-party scheduled file replacements from genuine
Windows servicing, but two issues remained:

1. `wsl --status` succeeding while the Host Compute Service is absent does not establish
   that a feature was enabled and is awaiting reboot. It can also mean the feature has
   never been installed. This now produces installation advice instead of a restart warning.
2. A saved `awaiting_restart` flag alone survives reboot indefinitely. Home and diagnostics
   were reading that flag directly, so a restart could leave the same warning visible.

Setup now timestamps restart requests. Home, diagnostics, and preflight compare the request
with Windows' boot timestamp. A confirmed later boot clears the presented warning and lets
setup continue with actual component/runtime validation; it does not mark setup complete.
Older state files use their modification time as the fallback request timestamp. Completed
setup cannot retain a stale warning.

The boot timestamp comes from Microsoft's documented
[`Win32_OperatingSystem.LastBootUpTime`](https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-operatingsystem)
through a hidden, five-second-bounded CIM query. Successful reads are cached for the process;
normal launches with no outstanding request do not make this query. Failed/unknown reads
retain the displayed request, but Continue setup can still validate the real components.
A confirmed same-boot request avoids repeated elevation; an unavailable CIM service must
not trap the user in a restart loop. No Windows registry state is changed.

The comparison uses UTC timestamps, so manually changing the system clock or modifying a
legacy state file's timestamp can affect the inference. Real reboot/resume still needs a
clean-Windows VM test; pure decision tests cannot establish those Windows feature paths.

## Tests and build

- Rust unit suite: **168 passed**, five integration tests separately selected.
- UI suite: **16 passed**. Covers direct launch with no notebook/workspace creation,
  retryable launch errors, setup, backups, workspace management, About, footer removal,
  themes, keyboard navigation, and compact-window layout.
- Production frontend build, tool TypeScript check, Knip, and strict Clippy: passed.
- Added/updated Rust coverage for empty folders, root-level notebook creation/import,
  collision preservation, legacy nested files, and restart requests across boot changes.
- Updated the real integration test to check that setup leaves an empty root and that the
  actual Home launch service opens JupyterLab after shutdown without creating extra files
  or directories. A second launch must reuse the same server.
- Real integration suite: **5 passed**. Fresh installation, direct JupyterLab access,
  same-server reuse, authenticated notebook access, and stop/restart completed in
  **290.65 seconds**. The other four checks passed in **11.38 seconds**. The diagnostic
  smoke test reported **healthy** despite unrelated pending file replacements. The
  runner successfully removed its disposable QA distribution afterward.
- **189 tests passed** in total. Formatting checks also passed.
- The 1.2.0 MSI build completed successfully, but before final file-table verification the
  shared repository's build artifacts were replaced. The expected
  `src-tauri/target/release/bundle/msi/SageDock_1.2.0_x64_en-US.msi` no longer exists. Do not
  apply this pass's test/build claims to the root-level 1.3.1 installer without checking it.
- After the unknown-boot-query recovery adjustment, all 168 Rust unit tests, strict Clippy,
  formatting checks, and the optimized Windows build passed again. The five integration
  tests above preceded that final adjustment, which affects the pending-restart branch.

The initial UI error test used an incomplete mock error and failed its expected-message
assertion. Adding the required `severity` field made it match the backend contract; the
full suite then passed. No production assertion was relaxed.

## Files changed in this pass

- UI: `src/pages/Home.tsx`, `src/components/WorkspaceCard.tsx`, `src/styles/layout.css`,
  `src/pages/Help.tsx`.
- Folder behavior: `src-tauri/src/runtime/workspace.rs`, `src-tauri/src/workspaces.rs`,
  `src-tauri/src/home.rs`, `src-tauri/src/jupyter/notebook.rs`, `src-tauri/src/library.rs`.
- Launch and restart handling: `src-tauri/src/desktop.rs`, `src-tauri/src/commands.rs`,
  `src-tauri/src/runtime/provision.rs`, `src-tauri/src/runtime/health.rs`,
  `src-tauri/src/system/mod.rs`, `src-tauri/src/system/reboot.rs`.
- Tests: `src-tauri/src/e2e.rs`, the backup legacy-layout fixture in `backup.rs`, tests
  beside the changed Rust logic, `tests/ui/workspace.spec.ts`, and refreshed `docs/qa/` images.
- Documentation: README, DESIGN, CODE-QUALITY, and this handoff.

This pass did not bump the source version or publish a release. Native installer
execution, clean-Windows component installation/reboot, and real WebView2 interaction
remain distinct from the tested browser UI and real WSL service paths.
