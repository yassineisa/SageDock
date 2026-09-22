# Security review guide

Written for a reviewer assessing SageDock for installation on shared university lab
computers. Every claim below names the file that implements it, so you can check the code
rather than trust this document. Where something is **not** protected, or not yet verified,
it says so.

SageDock's threat model is a shared lab machine with a non-administrator student account.
The assets worth protecting are the student's notebooks, the machine's other users, and the
lab network. SageDock is not a sandbox and does not claim to contain hostile code that a
student deliberately runs inside their own notebook.

## 1. The notebook server is not reachable from the network

This is the single most important property for a lab deployment, because JupyterLab grants
arbitrary code execution to anyone who can reach it with a valid token.

Implemented in [`src-tauri/src/jupyter/mod.rs`](../../src-tauri/src/jupyter/mod.rs):

- The server is launched with `--ServerApp.ip=127.0.0.1` and
  `--ServerApp.allow_remote_access=False`. It binds loopback only.
- The port is chosen by binding port `0` and letting the OS pick a free one
  (`find_free_port`). There is no fixed, guessable port.
- Every session gets a fresh 32-byte token from the OS CSPRNG via `getrandom::fill`,
  rendered as 64 hex characters (`generate_token`). Tokens are not reused between sessions
  and are never shown in the interface.
- `notebook_url` builds `http://127.0.0.1:<port>/lab/tree/<escaped>?token=<token>`. The
  token is in the URL because that is Jupyter's browser authentication mechanism; the URL
  is handed directly to the browser or webview.

Three unit tests guard this: `urls_are_always_loopback`, `tokens_are_long_random_hex`
(asserts length, hex, and non-repetition), and `allocated_ports_are_usable_and_not_fixed`.

`verify_kernels` additionally confirms that an **unauthenticated** request to
`/api/contents` is rejected, and treats anything other than a `401`/`403` as a failure. In
other words the code positively asserts that authentication is switched on, rather than
assuming it.

**Residual risk to weigh:** the token travels in a URL, so it can appear in browser history
when the student uses the "open in browser" preference. Any local process running as the
same user can also reach the loopback port and read that token from the URL. Loopback
binding does not isolate SageDock from other software in the same user session, and it is
not meant to.

## 2. No untrusted value is ever concatenated into a shell command

Commands are built as `argv` arrays and executed through `wsl.exe --exec`, which does not
involve a Linux shell.

- [`jupyter/mod.rs`](../../src-tauri/src/jupyter/mod.rs) builds `port_arg`, `token_arg` and
  `root_arg` as separate array entries and passes them through `wsl::exec_args`. The comment
  there records the reason: a workspace path containing spaces, quotes or `$` must reach
  Jupyter unchanged.
- [`runtime/health.rs`](../../src-tauri/src/runtime/health.rs) runs a **fixed** Python
  program and passes the workspace root as `argv[1]`, never interpolated into the source.
- The integration suite specifically covers shell-metacharacter paths (see
  [TESTING.md](TESTING.md)).

Windows-side processes are spawned with `CREATE_NO_WINDOW` so no console flashes on screen;
that is a polish decision, not a security boundary.

## 3. The webview cannot name a filesystem path

The frontend never originates an absolute path. This is worth checking carefully, because it
is the boundary that stops a compromised renderer from reading arbitrary files.

- [`src/lib/commands.ts`](../../src/lib/commands.ts) is the only place the frontend calls
  `invoke`. Its header states the rule and the type signatures enforce it: no command takes
  an absolute path.
- Native pickers run in **Rust** (`choose_notebook_file`, `choose_sage_package`,
  `add_workspace_folder`, `add_files_to_workspace` in
  [`desktop.rs`](../../src-tauri/src/desktop.rs) and
  [`home.rs`](../../src-tauri/src/home.rs)).
- A chosen backup, SageMath package or notebook is held in Rust state (`selected_backup`,
  `selected_package`, `selected_import` in [`state.rs`](../../src-tauri/src/state.rs))
  between preview and confirmation, deliberately so the path is not handed to the webview
  and passed back.
- Notebook paths are workspace-relative and re-resolved by `library::resolve` on every use.
- Files dropped onto the window are handled by the Rust window event in
  [`lib.rs`](../../src-tauri/src/lib.rs), not by the webview's own drop payload, so the
  absolute paths Windows reports never reach the frontend. The same applies in reverse for
  dragging a notebook out: `start_notebook_drag` takes the relative name the backend
  already handed out and resolves it in Rust.
- The browser choice added in 1.4.3 keeps the same rule. The frontend receives and returns
  a registry **identifier**, never a path; `browsers::open` in
  [`browsers.rs`](../../src-tauri/src/browsers.rs) re-resolves that identifier against
  `SOFTWARE\Clients\StartMenuInternet` to find the executable, and passes the URL as a
  single argument rather than through a shell.
- Downloads from the built-in browser, also added in 1.4.3, keep the rule as well. The file
  goes wherever the student picks in a native Save As dialog shown from Rust, and the path
  it landed at is recorded in [`downloads.rs`](../../src-tauri/src/downloads.rs), a record
  of what this application downloaded, never a scan of the disk. Home lists those files
  under an opaque `tracked:<hash-of-path>` key rather than their location, and
  `library::resolve_download` only ever matches such a key back against that recorded list,
  so the webview can neither learn where a download went nor name a file SageDock did not
  download. Rows display the containing folder's **name** only.

## 4. Capability and CSP surface

- [`src-tauri/capabilities/default.json`](../../src-tauri/capabilities/default.json) grants
  exactly `core:default`, to the window labelled `main` only. The `opener`, `dialog` and
  `single-instance` plugins are registered in Rust but **no plugin permission is granted to
  the webview**, so the frontend cannot ask the opener to launch an arbitrary URL. The
  GitHub address in `open_project_page` is a Rust constant.
- The CSP in [`tauri.conf.json`](../../src-tauri/tauri.conf.json) is restrictive:
  `default-src 'self'`, `script-src 'self'` (no `unsafe-inline`, no remote origins),
  `object-src 'none'`, `frame-src 'none'`, `base-uri 'self'`. `style-src` allows
  `'unsafe-inline'`, which is required because the UI sets a few inline style attributes.
- Notebook windows are separate webviews created with `WebviewUrl::External` and, as the
  header comment in `desktop.rs` records, receive no IPC capabilities.
- `allowed_navigation` pins each notebook window to `http`, host `127.0.0.1`, that
  session's exact port, and rejects embedded credentials. The unit test
  `notebook_navigation_is_restricted_to_this_session` checks five hostile URLs, including
  `http://localhost:8888` (a different host string) and `http://user@127.0.0.1:8888`.

## 5. It does not run as administrator

The application runs as the logged-in user. Elevation is confined to the one operation that
genuinely requires it: enabling the Windows features WSL 2 depends on, during setup. Review
[`runtime/provision.rs`](../../src-tauri/src/runtime/provision.rs) for the elevation call
site and its handling of exit code `1223` (the user cancelled the UAC prompt) and `3010`
(Windows requires a restart).

Nothing in the codebase disables Windows security features, alters Windows Update state, or
deletes reboot-related registry values. The restart detection in
[`system/reboot.rs`](../../src-tauri/src/system/reboot.rs) performs **read-only** registry
reads, and its module comment states this explicitly.

**Lab-relevant consequence:** on a locked-down lab image where students are not
administrators, the Windows-feature step cannot succeed from a student account. WSL 2 and
virtualization should be enabled in the base image by whoever builds it. See
[LAB-DEPLOYMENT.md](LAB-DEPLOYMENT.md).

## 6. It touches only its own WSL distribution

A lab machine may have unrelated WSL distributions belonging to other coursework.

- `wsl::terminate_distro` and `wsl::stop_checked` in
  [`runtime/wsl.rs`](../../src-tauri/src/runtime/wsl.rs) run
  `wsl --terminate <SageDock's own name>`. `wsl --shutdown` is never used anywhere in the
  tree; grep for it to confirm.
- `may_stop_runtime` in [`state.rs`](../../src-tauri/src/state.rs) stops the environment on
  exit only if this instance started it and nothing is in flight.
- The `single_instance` plugin ensures one SageDock owns the environment; a second launch
  focuses the first window and exits.
- The QA integration runner can only target a throwaway distribution: `distro_name()`
  asserts the override starts with `SageDockQA-` and is alphanumeric, and the override is
  compiled in under `#[cfg(test)]` only.

## 7. Supply chain of the bundled runtime

The installer embeds a ~1.4 GB SageMath image, which is the largest piece of third-party
code in the product.

- [`src-tauri/trusted-runtimes.json`](../../src-tauri/trusted-runtimes.json) pins the
  image's SHA-256, byte size and architecture.
- [`scripts/stage-runtime.ps1`](../../scripts/stage-runtime.ps1) refuses to stage an archive
  whose manifest is not in that pinned list, then independently re-hashes the file with
  `Get-FileHash` and re-checks its length before copying. An unpinned or altered image
  cannot enter a build.
- At runtime, `runtime/image.rs` verifies the package and `health.rs` asserts the image's
  `runtime.json` declares a compatible `format` version.

**Worth noting in review:** the pinned hash is the _only_ thing establishing that image's
provenance. Whoever accepts this build should confirm the hash corresponds to an image they
or a trusted party actually produced with
[`scripts/build-runtime.ps1`](../../scripts/build-runtime.ps1).

## 8. What leaves the machine

- The diagnostic export (`diagnostic_report` in
  [`desktop.rs`](../../src-tauri/src/desktop.rs)) emits only check ids, severities and
  plain-language summaries, plus a workspace **count**. It deliberately excludes raw command
  output, notebook names, home paths and server URLs, and carries an explicit `privacy`
  field saying so. Saving it is an explicit user action through a native save dialog.
- There is no telemetry, analytics or crash reporting in the tree. `ureq` is used only
  against `127.0.0.1` for Jupyter readiness and shutdown; grep the call sites to confirm.
- Setup installs from a **local** package, so a lab rollout needs no internet access for
  SageMath itself. `webviewInstallMode` is `downloadBootstrapper`, which _does_ reach
  Microsoft to fetch WebView2 if it is missing. Pre-install WebView2 in the base image to
  keep installation fully offline.

## 9. User data handling

- `forget_workspace` removes a folder from SageDock's list and never deletes it; the
  frontend type comment and the Rust implementation agree, and a UI test asserts it.
- Config and state writes go through `storage::atomic_write`, so an interrupted write cannot
  truncate an existing file.
- Notebooks live in ordinary Windows folders under `Documents\SageDock`, outside the Linux
  runtime, so repair and reinstall cannot take them with it.

**Two honest caveats**, both documented in
[`docs/BACKUP-FORMAT.md`](../BACKUP-FORMAT.md):

1. Backups are **unencrypted** ZIP files, chosen so a student can open one without
   SageDock.
2. Backups copy workspace files as-is. If a student leaves credentials in a notebook or a
   `.env` file inside a workspace, those are copied into the backup. Jupyter runtime
   directories, which hold live tokens, are excluded.

## 10. Gaps a reviewer should not overlook

- **The installer is not code-signed.** There is no `signingIdentity` or certificate
  configuration in `tauri.conf.json`, and no signing key in the tree (`.gitignore`
  pre-emptively blocks one). Students will see a SmartScreen warning, and a lab deployment
  should distribute the MSI through a channel that establishes provenance some other way.
- **No clean-machine verification.** Nothing in this repository has been installed and run
  on a fresh Windows image without WSL. The Windows-feature installation, UAC, reboot and
  resume paths are exercised by unit tests over pure decision logic, not by a real
  transition. See the limits section of [TESTING.md](TESTING.md).
- **Students execute arbitrary code by design.** SageMath and Python notebooks run whatever
  the student writes, inside WSL, as the `sagedock` Linux user, with the workspace folder
  mounted from Windows. That is the product's purpose, not a defect, but it means SageDock
  is a code-execution tool and should be deployed with that understood.
