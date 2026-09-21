# Deploying to lab computers

Written for whoever has to decide whether this can go on a managed Windows image, and what
it will cost them. Every fact about the installer below was read out of the MSI database of
`SageDock-1.3.1-Installer.msi`, not inferred from configuration.

## The installer at a glance

| Property              | Value                                                                  |
| --------------------- | ---------------------------------------------------------------------- |
| File                  | `SageDock-1.3.1-Installer.msi`, **1,469,722,624 bytes** (1,401.6 MB)   |
| ProductName / Version | `SageDock` / `1.3.1`                                                   |
| Manufacturer          | `Yassin Eisa`                                                          |
| ProductCode           | `{BF9DF498-1798-458C-9099-A75CA4E47CC0}`                               |
| UpgradeCode           | `{B05BF660-072E-5795-9B7F-9C42C518C5E8}`                               |
| Scope                 | `ALLUSERS=1` — **per-machine, requires administrator**                 |
| Install location      | `ProgramFiles64Folder\SageDock`                                        |
| Bundle type           | WiX MSI only (`bundle.targets` is `["msi"]`); there is no NSIS or MSIX |
| Code signing          | **None.** Expect SmartScreen.                                          |

The `UpgradeCode` is stable across versions and an `Upgrade` table is present, so a newer
MSI performs a major upgrade of an older one rather than installing side by side. Deploying
1.3.1 over an existing 1.2.0 is supported by the package; it has not been tested.

### What it puts on disk

Six file entries, nothing hidden:

| File                                        | Bytes         |
| ------------------------------------------- | ------------- |
| `sagedock.exe`                              | 7,888,384     |
| `sagedock-runtime-sage10.9-x64.tar.xz`      | 1,464,064,000 |
| `sagedock-runtime-sage10.9-x64.tar.xz.json` | 283           |
| `LICENSE`                                   | 1,068         |
| `THIRD-PARTY-NOTICES.md`                    | 3,385         |
| `README.txt`                                | 171           |

So `Program Files\SageDock` occupies roughly **1.4 GB**, almost all of it the compressed
SageMath runtime image, which is staged there as a resource and imported into WSL later.

### Shortcuts and Add/Remove Programs

- Start Menu: `SageDock`
- **Desktop: `SageDock`** — worth knowing if your image policy forbids desktop shortcuts.
- `Uninstall SageDock` inside the install directory.
- `ARPNOMODIFY` is set, so Add/Remove Programs offers no "Modify" button.

### Custom actions

The MSI declares six, and it is worth knowing exactly what they are:

`LaunchApplication`, `WixUIValidatePath`, `WixUIPrintEula`, `SetARPNOMODIFY`,
`SetARPINSTALLLOCATION`, and `DownloadAndInvokeBootstrapper`.

These are all standard WiX UI, ARP-registry and WebView2-bootstrapper actions. **None of
them touches WSL, and none removes user data.** That matters for the uninstall section
below.

## What the base image must provide

SageDock cannot create these for a non-administrator student, so they belong in the image:

1. **Windows 10 or 11, x64.** Checked at runtime by `system/windows_version.rs` and
   `system/architecture.rs`.
2. **WSL 2**, with the **Virtual Machine Platform** feature enabled.
3. **Hardware virtualization enabled in firmware.** Checked by
   `system/virtualization.rs`; if it is off, SageDock can only tell the student to ask an
   administrator.
4. **WebView2 runtime.** `webviewInstallMode` is `downloadBootstrapper`, and the
   `DownloadAndInvokeBootstrapper` custom action confirms it: if WebView2 is missing, the
   installer **reaches out to Microsoft to download it**. Pre-install WebView2 and the
   installation is fully offline. It is already present on current Windows 11.

If WSL 2 and the Virtual Machine Platform are absent, SageDock's setup tries to enable
them via DISM, which requires elevation and usually a restart. On a locked-down student
account that step cannot succeed. **Enable them in the image.** See
[RESTART-LOGIC.md](RESTART-LOGIC.md) for how SageDock distinguishes a restart that genuinely
blocks setup from ordinary Windows update noise.

## Capacity planning, which is the real constraint

This is the part most likely to cause trouble on a lab machine, so it deserves attention.

- **Program Files: ~1.4 GB**, once, machine-wide.
- **Setup demands 15 GB free** before importing the runtime. This is not a guess:
  `runtime/health.rs` calls `require_space(data, if importing { 15 } else { 1 })`, so
  preflight refuses to start an import with less, and asks for 1 GB otherwise.
- **The imported WSL distribution is per Windows user profile.** WSL registers
  distributions per user, so **every student who logs in and runs setup creates their own
  multi-gigabyte copy** of the SageMath environment, expanded from that 1.4 GB archive.

That last point is the one to think hardest about. On a shared lab machine with many
student profiles, or with roaming or temporary profiles, the disk cost multiplies by the
number of users, and a wiped profile means the next session re-imports from scratch (a
first-time setup measured around 250 seconds in the integration suite). SageDock was
designed for a student's own computer; nothing in it is per-machine-shared except the
installed archive.

If your lab uses non-persistent profiles, budget the import time per session or plan to
pre-seed the distribution as part of the image.

## Where user data lives

- **Notebooks:** ordinary Windows folders, by default `Documents\SageDock`, deliberately
  **outside** the Linux runtime so repair and reinstall cannot take them. Students can keep
  several workspaces, each a plain folder they can move or copy.
- **Settings:** `config.json` in the Tauri per-user app config directory for identifier
  `com.sagedock.desktop`.
- **Setup state:** `setup-state.json` in the per-user app data directory.
- **Logs:** JSON lines, daily-rotating, in the per-user app log directory. `info` level by
  default; verbose only if someone sets `SAGEDOCK_LOG=debug`.

Note the Documents resolution chain in `lib.rs`: on a machine with OneDrive-redirected
Documents the Windows known-folder API can return an empty path, and SageDock falls back
rather than dropping notebooks into AppData. Lab images that redirect Documents to a
network share should be tested, because notebooks would then live on that share and the
Windows-to-Linux path bridge has only been exercised against local paths.

## Uninstall

The MSI removes `Program Files\SageDock`, the shortcuts and the ARP entry. Based on the
custom-action table, **it does not remove**:

- the imported WSL distribution (multiple gigabytes, per user), or
- the student's notebooks in `Documents\SageDock`, or
- per-user settings, state and logs.

Leaving notebooks alone is deliberate and correct: the product treats student work as more
valuable than its own environment, and nothing deletes a project folder without explicit
confirmation. But it does mean **uninstalling does not reclaim the disk**. To fully clean a
machine you must additionally unregister the distribution per user, which SageDock exposes
through Recovery, or do it with `wsl --unregister <name>`.

SageDock only ever terminates or unregisters **its own** distribution; `wsl --shutdown` is
used nowhere in the codebase. Unrelated WSL distributions belonging to other coursework are
safe. See [SECURITY.md](SECURITY.md) §6.

## Network

- **SageMath itself needs no network.** The runtime is installed from the archive inside the
  installer, which is why the installer is 1.4 GB.
- The notebook server binds `127.0.0.1` only and is never exposed to the lab network. This
  is the most important security property; see [SECURITY.md](SECURITY.md) §1.
- No telemetry, analytics or crash reporting exists in the codebase.
- Outbound connections at install time are limited to the WebView2 bootstrapper, and only
  if WebView2 is absent.
- Optional extras in the Tools page (C++, Fortran, seaborn, statsmodels, polars) install
  from inside the Linux environment and **do** need network access when used.

## Before you approve a rollout

Honest list of what has not been established. None of it is hidden elsewhere in the repo.

- [ ] **Install the MSI on a clean Windows 10 and 11 image.** The package was verified by
      reading its database, not by running it. No installation has ever been performed by a
      test.
- [ ] **Test with WSL 2 absent**, so the DISM, UAC, restart and resume path actually runs.
      That path is covered only by unit tests over pure decision logic.
- [ ] **Test from a non-administrator student account**, which is the real deployment
      condition and the one most likely to fail.
- [ ] **Test with virtualization disabled in firmware**, to confirm the guidance is
      actionable rather than a dead end.
- [ ] **Confirm the UI in real WebView2.** All 16 UI tests run in Edge against a mocked IPC
      bridge, so nothing has exercised the packaged shell or any native file dialog.
- [ ] **Decide about code signing.** The MSI is unsigned; distribute it through a channel
      that establishes provenance.
- [ ] **Verify the runtime image's provenance** against the SHA-256 in
      `src-tauri/trusted-runtimes.json`. That hash is the only thing attesting to what is
      inside the 1.4 GB environment you would be deploying to students.
- [ ] **Test a 1.2.0 → 1.3.1 major upgrade** if any machine already has an earlier build.

The remaining clean-Windows, native-dialog and fault-injection items are tracked in
[QA-REVIEW.md](../../coding%20agent%20onboarding/QA-REVIEW.md).
