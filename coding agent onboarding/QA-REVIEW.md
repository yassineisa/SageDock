# Review and corrections — 17 September 2026

This is the current handoff for the review requested after the Home shutdown, portable
backup, workspace cards, and licensing implementation. Read `app.md` for product scope.
`IMPLEMENTATION.md` records the preceding agent's implementation; this document supersedes
its behavior descriptions and verification claims where they differ.

## Corrections

### Stop and restart

- Moved **Stop SageMath** directly below the Home introduction, above notebook actions and
  workspace cards. Preserved the logo, visual language, and save-before-stopping warning.
- Home previously ran a Linux health check immediately after stopping, which started WSL
  again. `commands::setup_status` now checks an already-running runtime only. Launch still
  checks health before starting Jupyter. The operation guard prevents a health check from
  racing an explicit stop.
- A failed WSL status query now produces an unknown state, rather than claiming the
  environment is stopped. Stop requires a successful observed stopped state and verifies
  the distribution belongs to this SageDock installation before terminating it.
- Jupyter process lifetime is separate from HTTP readiness. Opening another course no
  longer kills a server just because its HTTP response was temporarily slow. Reopening an
  unresponsive server offers retry/stop guidance. Rename remains blocked while any tracked
  notebook process is alive, even if its HTTP service is unresponsive.

### Portable backup and restore

- Home exposes backup, restore, and folder management before SageMath setup and when runtime
  checks fail. Only notebook launch depends on runtime readiness. Independent Home reads
  use `Promise.allSettled`, so an unreadable recent-notebooks folder does not hide the other
  controls.
- Bounded manifest decompression to 128 MiB. Recomputed manifest totals with checked
  arithmetic; enforced the 96 GiB / 400,000-file archive and 8 GiB per-file limits.
- Rejected Windows device names, streams, invalid path characters, trailing dots/spaces,
  case-insensitive duplicate paths, file/directory collisions, duplicate workspace slugs,
  and unsupported format versions.
- Extraction compares ZIP sizes with manifest sizes before writing, bounds every stream
  by its declared size, verifies SHA-256, exclusively creates output files, and syncs them.
- Restore stages **all** workspaces before publishing any. A damaged later workspace can
  no longer leave earlier workspaces published but absent from the list.
- `restore_with_commit` includes saving the workspace registry in its transaction. Handled
  extraction, publication, or registry errors roll back only paths this operation created.
  Windows can prevent cleanup; the error text acknowledges this and logs the failure.
- Replaced predictable, destructively reused staging paths with random, exclusively created
  paths. Pre-existing partial files and folders are preserved.
- Restore naming considers actual folders on disk as well as registered names. It preserves
  unregistered coursework and keeps suffixed names within Windows workspace-name limits.
  Preview uses the same destination directory. A concurrent filesystem change can still
  require a different free name at restore time.
- Backups cannot be saved inside a workspace. Missing/unreadable workspaces and depth-limit
  violations cause an explicit failure instead of a misleading partial success.
- Source growth is bounded while copying; a final rescan checks additions, removals, size,
  and modification times before publication. SHA-256 verification reads the archive back.
- Windows CLOUD reparse tags are allowed so OneDrive placeholders are not silently omitted.
  Other reparse points are refused. Reading cloud files may need the provider/network;
  make files available offline first. This branch still needs real OneDrive validation.

### Workspace integrity

- Removed/renamed default workspaces no longer reappear after every app restart.
- Unreadable or newer-version workspace lists are preserved and protected against writes.
  Mutations fail with a specific message before creating or moving coursework. A support
  workflow for recovering a corrupt list remains a future improvement.
- Missing active folders retain their identity. Notebook operations fail with reconnect/
  select-another-course guidance instead of falling back to a different course's files.
- New folders are created exclusively. Registry-save failure removes only the empty folders
  just created, never recursively deleting newly arrived user or sync-client files.
- Rename rolls the folder back if registry persistence fails. If Windows also blocks that
  rollback, the error identifies the retained folder for **Add existing folder** recovery.
- Renaming a parent updates registered child workspace paths.
- Launch sets the active workspace only after its server and viewer open successfully.

### Licensing and packaging metadata

- Verified the repository's MIT text and `Copyright (c) 2026 Yassin Eisa` attribution.
- Added Yassin Eisa/MIT to Cargo and npm metadata and synchronized the npm lockfile's root
  version with 1.1.0.
- Added publisher, copyright, license-file, and notice resources to Tauri packaging.
- Third-party components retain their own licenses. This review is not a complete audit of
  the scientific runtime's distribution notices or source-offer requirements.
- Added generated QA folders to `.gitignore`; preserved source files and screenshots.

## Redesign and 1.1.1 packaging — 17 September 2026

Added after the review above, at the maintainer's request.

- The interface was rebuilt on an editorial "Lab Ledger" system: paper and ink, hairline
  rules instead of floating cards, a left-set asymmetric column, and one accent (`#8c2f20`)
  reserved for what is active or wrong. Zero border radius, zero shadows, no colour
  transitions; hover inverts instantly. Typefaces are Windows-bundled (Sitka, Segoe UI,
  Consolas) because the CSP is `default-src 'self'` with no `font-src`, so a hosted webfont
  would be blocked at runtime and bundling one would enlarge a 1.4 GB installer for nothing.
  The full contract is in [`DESIGN.md`](../docs/DESIGN.md).
- Presentation only. `theme.css` and `layout.css` were rewritten and the decorative artwork
  was removed from `Home.tsx`. No command, serialized shape, or safety rule changed.
- Versions bumped to **1.1.1** in `Cargo.toml`, `tauri.conf.json`, and `package.json`.
- Screenshots in `docs/qa/` were regenerated and visually inspected in light and dark.

## Windows redesign (1.1.3) — 17 September 2026

**Supersedes the "Lab Ledger" description in the section above.** That editorial direction
was replaced at the maintainer's request; `DESIGN.md` describes only the current system.

Rebuilt on **Metro's principles** with **Windows 11 / Fluent** conventions for the
specifics, from Microsoft's own documentation rather than recollection:

- Segoe UI Variable with the documented type ramp (12/16 caption → 28/36 title), Regular and
  Semibold only — Bold and Italic are deliberately absent from the Windows ramp — and
  sentence case throughout.
- **4px** corner radius on persistent controls, **8px** on transient surfaces. Windows 11
  geometry guidance, not preference.
- **Segoe Fluent Icons** as the single icon family, via `src/components/Icon.tsx`. It ships
  with Windows, so nothing is bundled, the CSP needs no `font-src`, and there is no
  third-party licence obligation. Codepoints are verified against Microsoft's font listing
  and kept in one table, because a wrong private-use codepoint renders as an empty box
  rather than failing loudly.
- Accent is **Concordia burgundy `#912338`** over warm iron neutrals. Status colours are
  separate from the accent, and no state is signalled by colour alone.
- Removed: marketing headlines, motivational copy, serif typography, decorative artwork,
  pill controls, heavy shadows, and Unicode symbols used as icons. Translucency was
  declined — WebView2 has no reliable Mica/Acrylic backdrop, so opaque is the honest choice.

### Behaviour preserved

This was a presentation change. No command, serialized shape, or safety rule was altered,
and the reviewed fixes above were carried across verbatim and are still asserted by tests:

- `Promise.allSettled` on Home, so one failing read cannot hide backup, restore, or folder
  management.
- `running == null` remains a distinct third state; a failed status query is never reported
  as "stopped". Stop stays enabled when running **or** unknown.
- The environment strip and Stop are gated on `environment_installed`, not on setup being
  complete, so Stop survives a failing health check.
- Workspaces and backups render before setup; only notebook launch requires readiness.
- Every destructive confirmation kept its wording, including the save-your-work warning.

### Defects found by inspecting renders, not by tests

All eleven UI tests passed while each of these was present. They were caught by looking at
the screenshots, which is why that step is not optional:

1. **Dark accent failed contrast.** A lightened burgundy (`#c4596e`) with a near-black label
   measured roughly 3.8:1, under the 4.5:1 minimum. Fixed by splitting the accent into a
   fill token (deeper, white text) and a foreground token (lighter, for icons and markers) —
   the two have opposite contrast requirements on dark.
2. **`auto-fill` reserved phantom grid columns**, so two Backups cards huddled left with dead
   space beside them. Changed to `auto-fit`.
3. **Theme switching flashed unreadable controls.** `.btn` transitioned `background` but not
   `color`, so on switching, white labels sat on a still-white fill for the length of the
   transition. Filled controls are no longer transitioned; motion remains only where the
   default background is transparent.
4. **Hover outranked selection.** `:hover:not(:disabled)` scores (0,3,1) against
   `[aria-pressed=true]`'s (0,2,1), so pointing at the chosen segmented option repainted it
   as unselected — on the one control whose purpose is showing which value is active.

### Accessibility

The navigation pane collapses to a 64px icon rail below 900px. Each link carries an explicit
`aria-label`, because `display:none` on the label would otherwise strip the accessible name
at exactly the declared minimum window size (`minWidth: 860`). Icons are `aria-hidden`
throughout, so glyphs never leak into accessible names.

### Screenshots

Regenerated by the UI tests into `docs/qa/` and visually inspected: Home, setup, and Settings
in both themes, About in dark, and Home at the 860×560 minimum. These prove browser layout
and event wiring only — not native dialogs, not WSL.

### Changed files (1.1.3)

**New:** `src/components/Icon.tsx`.

**Rewritten:** `src/styles/theme.css`, `src/styles/layout.css`, `src/components/AppShell.tsx`,
`src/components/NavRail.tsx`, `src/components/WorkspaceCard.tsx`,
`src/components/ErrorBanner.tsx`, `src/components/ConfirmDialog.tsx`, `src/pages/Home.tsx`,
`src/pages/Settings.tsx`, `src/pages/Tools.tsx`, `src/pages/Recovery.tsx`,
`src/pages/Diagnostics.tsx`, `src/pages/Help.tsx`, `docs/DESIGN.md`.

**Modified:** `tests/ui/workspace.spec.ts` (two screenshot-capture tests added; every existing
behavioural assertion kept unchanged), `docs/QA-REVIEW.md`, `docs/IMPLEMENTATION.md`, and the
four version files — `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `package.json`,
`package-lock.json` — all at **1.1.3**.

**Untouched:** every Rust source file, `src/lib/commands.ts`, `src/state/*`, and the logo.
No backend contract changed, so no Rust check was affected by the redesign itself.

### Limitations of this redesign

- **Verified only in mocked Chromium/Edge, never in the real app.** Playwright drives the
  frontend with every IPC call stubbed. Nothing here has been seen running in WebView2 inside
  the packaged desktop application, where native dialogs, the real title bar, DPI scaling, and
  actual window resizing behave differently. Treat the screenshots as layout evidence, not as
  proof the desktop app looks or behaves this way.
- **Segoe Fluent Icons is assumed present.** It ships with Windows 11; Windows 10 falls back
  to Segoe MDL2 Assets, which shares most but not all of these codepoints. The fallback has
  not been checked on a Windows 10 machine, and a missing glyph renders as an empty box
  rather than failing loudly.
- **`src/components/EmptyState.tsx` and `src/components/HelpTip.tsx` are now orphaned.** They
  still typecheck but reference class names the new stylesheet no longer defines. They are
  imported nowhere; delete them or restyle them before reuse.
- **Translucency was declined,** not deferred. WebView2 exposes no reliable Mica or Acrylic
  backdrop, so the app is opaque in both themes by choice.
- **Contrast was reasoned about, not measured with a tool.** The accent split came from an
  estimate that the previous dark pairing sat near 3.8:1; a formal audit across every
  token pair has not been run.
- The two screenshot-capture tests assert almost nothing — they exist to produce evidence.
  The behavioural coverage is the other nine.

## Validation

Commands run on this Windows development computer:

| Check                                                                      | Result                                                                                             |
| -------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `cargo test --offline --lib --manifest-path src-tauri/Cargo.toml`          | 158 passed, 0 failed, 5 ignored                                                                    |
| `cargo check --offline --all-targets --manifest-path src-tauri/Cargo.toml` | Passed without warnings                                                                            |
| `npm.cmd run build`                                                        | TypeScript and production Vite build passed                                                        |
| `npm.cmd run test:ui`                                                      | 11 passed in installed Edge; IPC is mocked (9 behavioural, plus 2 that capture the screenshot set) |
| `scripts/test-fresh-install.ps1`                                           | Passed, including Home refresh after stop and kernel execution after restart (285.61 seconds)      |

The real integration test creates a unique `SageDockQA-*` distribution from the verified
runtime package. It checks setup's Sage and Python notebook execution, authenticated
Jupyter access, refusal of unauthenticated requests, two simultaneous workspace servers,
termination, preservation of the saved notebook, and restart. Cleanup validates the QA
name and registration path before unregistering it. The personal SageDock distribution is
not a test target. The initial successful run took about 271 seconds.

The test was extended to call the actual `commands::setup_status` after stop and assert
that Home refresh leaves WSL stopped; it also executes both kernels again after restart.
See the final validation entry below for that run's result.

New regression tests cover manifest expansion limits, incorrect totals, Windows aliases,
duplicate/file-directory paths, damage in a second workspace, registry commit failure,
pre-existing staging files, unregistered folder collisions, bounded restored names,
backup destination containment, depth-limit failures, default-workspace removal, missing
active-workspace identity, newer store preservation, and nested workspace rename.

Screenshots in `docs/qa/` were regenerated by the UI tests; Home was visually inspected.
These prove browser layout and event wiring, not native dialog behavior or WSL operation.

## Remaining validation before public distribution

1. Test the packaged installer on clean Windows 10/11 VMs with WSL absent: UAC, component
   download failure, restart/resume, and virtualization disabled. This PC already has WSL;
   installing a new distribution does **not** prove those Windows-component paths.
2. **Install** a new MSI on a clean machine. One _has_ now been built from this source:
   `SageDock_1.2.0_x64_en-US.msi`, 1,469,779,968 bytes (1,401.7 MB), 17 September 2026,
   verified by reading the package back rather than trusting its filename. Its
   `ProductVersion` is 1.2.0, its manufacturer is Yassin Eisa, and it carries six file
   entries: the SageMath archive at 1,464,064,000 bytes, matching the size pinned in
   `trusted-runtimes.json`, that archive's checksum manifest, the executable, `README.txt`,
   `LICENSE`, and `THIRD-PARTY-NOTICES.md`. It lives at
   `Desktop\SageDock-1.2.0-Installer.msi`, moved there rather than copied, so no
   duplicate remains in the repository. Building a package and reading it back is not
   installing it: the installation itself remains untested, and the interface has never
   been seen running in WebView2, only in mocked Edge. See
   [CODE-QUALITY.md](CODE-QUALITY.md) for that build's full gate results. The four older
   installers still on the Desktop contain earlier implementations.
3. Exercise native backup pickers and real OneDrive offline/online files, removable-drive
   disconnects, disk exhaustion, and large coursework collections on a second Windows user.
4. Test abrupt process/power loss. Restore rollback covers handled errors, not a journaled
   multi-directory transaction across crashes. A crash can leave staging folders or an
   unregistered completed folder. Original backups and existing coursework are preserved;
   retry restores alongside it, or reconnect a complete folder through **Add existing folder**.

## Current limitations and continuation notes

- Portable backups contain file contents and workspace identity. Empty directories have no
  archive entries. Stored preference fields are reserved metadata; restore intentionally
  leaves the receiving installation's appearance/browser preferences unchanged.
- Backup is not a live filesystem snapshot. Save/close notebooks first; size/mtime checks
  cannot detect every possible same-size edit with a preserved timestamp. Archives are
  ordinary unencrypted ZIPs; user-file contents can include personal information or secrets.
- Corrupt/newer workspace-list protection needs a guided recovery UI. Do not “fix” it by
  silently overwriting the unreadable list.
- The earlier runtime-repair backup retention and setup-completed-flag concerns in
  `IMPLEMENTATION.md` were outside this feature review and remain open for a focused audit.
- The repository has no commits, so there is no trustworthy git diff baseline. The
  historical one-time source-rewriting scripts were removed during the subsequent
  [code-quality cleanup](CODE-QUALITY.md), which records current test results.
- Run Rust builds/tests sequentially with the real integration executable on Windows;
  rebuilding the same executable while it runs causes linker error LNK1104.

## Final integration validation

Passed on 17 September 2026 in **285.61 seconds**. Setup reached Ready in 250 seconds.
The final assertions confirmed:

- Two authenticated Jupyter servers served separate workspace roots concurrently.
- Unauthenticated notebook API access was refused.
- The QA distribution stopped; the actual Home setup-status command returned successfully
  without starting it again.
- The saved notebook remained on Windows storage.
- Reopening the workspace restarted Jupyter, and both Sage and Python executed notebook
  checks again successfully.
- The test script unregistered its disposable QA distribution successfully.

A 1.2.0 installer has since been built and verified as described in item 2 above, but it
has not been installed anywhere. The clean-Windows VM checks listed above remain necessary
before distribution, and the redesigned interface still needs to be seen in the packaged
desktop application rather than in a browser with mocked IPC.

Rust coverage was re-run at 1.2.0 and stands at **155 passed, 0 failed, 5 ignored** out of 160. The figure of 158 previously recorded here predated the code-quality cleanup, which
replaced five tests of disconnected legacy helpers with two covering the exit-code
classifier used by production installation; see [CODE-QUALITY.md](CODE-QUALITY.md). The
1.1.3 redesign itself touched no Rust source, but the later About attribution change added
one command, `open_project_page`.
