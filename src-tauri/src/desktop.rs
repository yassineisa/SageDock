//! User workflows composed from runtime services. Remote notebook windows receive no IPC capabilities.
use crate::jupyter::JupyterSession;
use crate::{
    error::{AppError, AppResult},
    library,
    runtime::{self, wsl},
    scientific,
    state::{busy_error, operation, AppState},
};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub fn set_browser_preference(
    value: bool,
    state: State<'_, AppState>,
) -> AppResult<crate::config::AppConfig> {
    state.update_config(|c| c.open_in_browser = value)
}

#[tauri::command(async)]
pub fn recent_notebooks(state: State<'_, AppState>) -> AppResult<Vec<library::NotebookEntry>> {
    library::recent(&state.active_workspace_dir())
}

/// Opens a native picker for a notebook to add to a workspace. Nothing is copied yet, only
/// remembered, so the student can be asked which workspace it belongs in, and about a name
/// collision if there is one, the same way a notebook found in Downloads is handled.
#[tauri::command(async)]
pub fn choose_notebook_file(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Option<String>> {
    let Some(file) = app
        .dialog()
        .file()
        .set_title("Choose a notebook to add to a workspace")
        .add_filter("Jupyter notebook", &["ipynb"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = file.into_path().map_err(|e| ui_error(e.to_string()))?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported Notebook.ipynb")
        .to_string();
    state.set_selected_import(path);
    Ok(Some(name))
}

/// Sends a notebook already inside the active workspace to the Recycle Bin.
///
/// Only reachable from the Recently Opened list, which only ever shows notebooks under the
/// active workspace, `library::resolve` is what confirms that and refuses anything else.
#[tauri::command(async)]
pub fn delete_recent_notebook(path: String, state: State<'_, AppState>) -> AppResult<()> {
    let workspace = state.require_active_workspace()?;
    let file = library::resolve(&workspace, &path)?;
    library::move_to_recycle_bin(&file)
}

/// Reveals a notebook already inside the active workspace in File Explorer, selected.
#[tauri::command(async)]
pub fn reveal_recent_notebook(
    path: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let workspace = state.require_active_workspace()?;
    let file = library::resolve(&workspace, &path)?;
    app.opener()
        .reveal_item_in_dir(&file)
        .map_err(|e| ui_error(e.to_string()))
}

/// Sends a downloaded file to the Recycle Bin, and stops listing it on Home.
#[tauri::command(async)]
pub fn delete_downloaded_file(
    name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let dir = downloads_dir(&app).unwrap_or_default();
    let file = library::resolve_download(&dir, &state.tracked_downloads(), &name)?;
    library::move_to_recycle_bin(&file)?;
    // A recorded download that has gone to the Recycle Bin must stop appearing on Home.
    state.forget_download(&file);
    Ok(())
}

/// Opens a downloaded file with whatever program Windows already associates with it, without
/// bringing it into SageDock, a quick look, not an import.
#[tauri::command(async)]
pub fn open_downloaded_file_externally(
    name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let dir = downloads_dir(&app).unwrap_or_default();
    let file = library::resolve_download(&dir, &state.tracked_downloads(), &name)?;
    app.opener()
        .open_path(file.to_string_lossy(), None::<&str>)
        .map_err(|e| ui_error(e.to_string()))
}

/// Reveals a downloaded file in File Explorer, selected.
#[tauri::command(async)]
pub fn reveal_downloaded_file(
    name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let dir = downloads_dir(&app).unwrap_or_default();
    let file = library::resolve_download(&dir, &state.tracked_downloads(), &name)?;
    app.opener()
        .reveal_item_in_dir(&file)
        .map_err(|e| ui_error(e.to_string()))
}

#[tauri::command(async)]
pub fn open_notebook(path: String, app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let _operation = state
        .begin_operation(operation::NOTEBOOK)
        .ok_or_else(|| busy_error(&state))?;
    let session = prepare_notebook_session(&state, &path)?;
    open_in_viewer(&app, &state, &session, ViewerTarget::Path(&path))
}

/// What a viewer window should show.
///
/// These are genuinely different screens, not one screen with an optional argument:
/// `Launcher` is JupyterLab's own landing page, while `Path` addresses a folder or a file
/// inside the session's root. Collapsing them is what made the Home button open a file
/// browser while advertising itself as the way to start working.
#[derive(Clone, Copy)]
pub enum ViewerTarget<'a> {
    Launcher,
    /// A notebook, or the session root when empty.
    Path(&'a str),
}

/// Opens JupyterLab's landing page, independent of whichever course was opened last.
///
/// Rooted at the default SageDock folder rather than the active workspace: this is the
/// general "start working" action, and a student who last launched Calculus should not find
/// that Home has quietly become a Calculus-only button. Workspace cards remain the way to
/// open a specific course. The session root still bounds the server, so "no specific path"
/// means the top of the SageDock folder and never the whole filesystem.
#[tauri::command(async)]
pub fn open_jupyter_home(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let _operation = state
        .begin_operation(operation::NOTEBOOK)
        .ok_or_else(|| busy_error(&state))?;
    let root = state.paths.workspace_dir.clone();
    runtime::workspace::ensure_workspace(&root)?;
    runtime::health::fast(&root)?;
    let session = state.session_for(&root)?;
    open_in_viewer(&app, &state, &session, ViewerTarget::Launcher)
}

/// Shared launch path for an existing notebook or the Home JupyterLab button. The caller
/// holds the operation guard; an empty path neither creates a notebook nor changes folders.
pub(crate) fn prepare_notebook_session(state: &AppState, path: &str) -> AppResult<JupyterSession> {
    let workspace = state.require_active_workspace()?;
    // Empty path opens JupyterLab's file browser, where projects remain ordinary folders.
    if !path.is_empty() {
        library::resolve(&workspace, path)?;
    }
    runtime::health::fast(&workspace)?;
    state.session_for(&workspace)
}

/// Shows a Jupyter session, either in a dedicated SageDock window or the default browser.
///
/// Shared with the workspace launcher so both honour the same preference and the same
/// navigation restrictions.
pub fn open_in_viewer(
    app: &AppHandle,
    state: &AppState,
    session: &JupyterSession,
    target: ViewerTarget<'_>,
) -> AppResult<()> {
    // Both screens are addressed the same way, inside this session's own Lab workspace,
    // and `session_url` is what decides whether a `/tree` segment belongs in the result.
    // Appending one unconditionally is what made launching a course land on
    // `/lab/workspaces/sagedock-<port>/tree`, which JupyterLab reports as not found.
    let path = match target {
        ViewerTarget::Launcher => "",
        ViewerTarget::Path(path) => path,
    };

    let config = state.get_config();
    if config.open_in_browser {
        // Separate browser tabs must not reset or compete for an existing Lab workspace.
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|e| ui_error(e.to_string()))?;
        let id: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let url = crate::jupyter::session_url(session, &format!("sagedock-{id}"), path);

        // A browser the student picked during setup, if they picked one. A browser that has
        // since been uninstalled falls back to the Windows default rather than failing: not
        // being able to honour a preference is not a reason to refuse to open their work.
        if let Some(chosen) = config.preferred_browser.as_deref() {
            match crate::browsers::open(chosen, &url) {
                Ok(()) => return Ok(()),
                Err(err) => tracing::warn!(
                    target: "launcher",
                    browser = chosen,
                    code = %err.code,
                    "chosen browser could not be started; using the Windows default instead"
                ),
            }
        }

        app.opener()
            .open_url(&url, None::<&str>)
            .map_err(|e| ui_error(e.to_string()))?;
        return Ok(());
    }

    let script = viewer_script(session.port, path);
    // Different server roots must not share the default persisted JupyterLab UI workspace.
    let url =
        crate::jupyter::session_url(session, &crate::jupyter::lab_workspace(session.port), path);
    let parsed: tauri::Url = url
        .parse::<tauri::Url>()
        .map_err(|e| ui_error(e.to_string()))?;
    // One window per session, so switching workspaces opens a second window rather than
    // navigating the first away from someone's running calculation.
    let label = format!("notebook-{}", session.port);
    if let Some(window) = app.get_webview_window(&label) {
        window.eval(&script).map_err(|e| ui_error(e.to_string()))?;
        window.set_focus().map_err(|e| ui_error(e.to_string()))?;
    } else {
        let port = session.port;
        WebviewWindowBuilder::new(app, label, WebviewUrl::External(parsed))
            .title("SageDock · Notebooks")
            .inner_size(1200.0, 820.0)
            // Downloading from the built-in browser asks where to save, the way any browser
            // does, instead of dropping the file into Downloads unannounced.
            //
            // This uses rfd's synchronous dialog rather than `tauri-plugin-dialog`, and that
            // is forced rather than preferred: WebView2 raises this on the UI thread, where
            // the plugin's `blocking_*` pickers deadlock the event loop by its own
            // documentation, while its callback form answers long after this handler has had
            // to return a destination. See the `rfd` note in Cargo.toml.
            .on_download(|webview, event| match event {
                tauri::webview::DownloadEvent::Requested { destination, .. } => {
                    // WebView2 has already filled in the name and folder it would have used,
                    // which makes the right defaults to open the dialog on.
                    let suggested = destination
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("download")
                        .to_owned();
                    let mut dialog = rfd::FileDialog::new()
                        .set_title("Save download")
                        .set_file_name(suggested.as_str());
                    if let Some(parent) = destination.parent() {
                        if parent.is_dir() {
                            dialog = dialog.set_directory(parent);
                        }
                    }
                    match dialog.save_file() {
                        Some(chosen) => {
                            *destination = chosen;
                            true
                        }
                        // Closing the dialog cancels the download rather than saving it
                        // somewhere the student never chose.
                        None => false,
                    }
                }
                tauri::webview::DownloadEvent::Finished { path, success, .. } => {
                    // Recorded only on success, and only with the path it actually landed
                    // at, so Home can list it even though it is outside Downloads.
                    if let (true, Some(path)) = (success, path) {
                        if let Some(state) = webview.try_state::<AppState>() {
                            state.record_download(path);
                        }
                    }
                    true
                }
                _ => true,
            })
            .on_navigation(move |url| allowed_navigation(url, port))
            .on_page_load(move |window, payload| {
                if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
                    && allowed_navigation(payload.url(), port)
                {
                    if let Err(err) = window.eval(&script) {
                        tracing::warn!(target: "jupyter", %err, "could not open notebook view");
                    }
                }
            })
            .build()
            .map_err(|e| ui_error(e.to_string()))?;
    }
    Ok(())
}

/// JSON serialization keeps notebook names as data, including quotes and script syntax.
pub(crate) fn viewer_script(port: u16, path: &str) -> String {
    let request = serde_json::json!({ "origin": format!("http://127.0.0.1:{port}"), "path": path });
    format!(
        "({})({request})",
        include_str!("jupyter/viewer.js")
            .trim()
            .trim_end_matches(';')
    )
}

fn allowed_navigation(url: &tauri::Url, port: u16) -> bool {
    url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port() == Some(port)
        && url.username().is_empty()
        && url.password().is_none()
}

/// Which list a dragged notebook came from.
///
/// Downloads sit outside every workspace, so the two resolve against different roots. Both
/// take only the same name the backend handed out, never a path chosen by the webview.
#[derive(serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum DragSource {
    Workspace,
    Downloads,
}

pub(crate) fn downloads_dir(app: &AppHandle) -> AppResult<std::path::PathBuf> {
    app.path().download_dir().map_err(|err| {
        AppError::new(
            "notebook",
            "DOWNLOADS_UNAVAILABLE",
            "SageDock couldn't find your Downloads folder",
            "Windows didn't report where your Downloads folder is, so SageDock can't list notebooks from it. Your other notebooks are unaffected.",
        )
        .with_technical_details(err.to_string())
    })
}

/// Downloads the student might want, most recently changed first.
///
/// Two sources: notebooks sitting in the Windows Downloads folder, and files the built-in
/// browser downloaded wherever they chose to save them.
#[tauri::command(async)]
pub fn downloaded_notebooks(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Vec<library::NotebookEntry>> {
    // Not being able to locate the Downloads folder must not hide downloads recorded
    // elsewhere, so this degrades to listing only those rather than failing outright.
    let dir = downloads_dir(&app).unwrap_or_default();
    library::downloaded(&dir, &state.tracked_downloads())
}

/// Starts a native drag so a notebook can be dropped onto the desktop or a folder.
///
/// The drag copies rather than moves: dragging a notebook out of SageDock must never take it
/// out of the student's workspace. The absolute path is resolved here and never crosses the
/// IPC boundary, which is why this takes a source and a name rather than a path.
#[tauri::command(async)]
pub fn start_notebook_drag(
    path: String,
    source: DragSource,
    app: AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let file = match source {
        DragSource::Workspace => {
            let workspace = state.require_active_workspace()?;
            library::resolve(&workspace, &path)?
        }
        DragSource::Downloads => library::resolve_download(
            &downloads_dir(&app).unwrap_or_default(),
            &state.tracked_downloads(),
            &path,
        )?,
    };
    // Embedded at compile time, so the drag image exists whatever the install layout is.
    let image = drag::Image::Raw(include_bytes!("../icons/32x32.png").to_vec());
    // The OS drag must begin on the thread that owns the window.
    app.run_on_main_thread(move || {
        if let Err(err) = drag::start_drag(
            &window,
            drag::DragItem::Files(vec![file]),
            image,
            |_result, _cursor| {},
            drag::Options::default(),
        ) {
            tracing::warn!(target: "desktop", %err, "could not start the drag");
        }
    })
    .map_err(|e| ui_error(e.to_string()))
}

/// The installed-tool report.
///
/// `force` is the user explicitly choosing "Check now". Without it a stopped environment is
/// left stopped and the last verified result is returned instead, labelled as cached, so
/// that opening a screen can never restart something the user deliberately stopped.
#[tauri::command(async)]
pub fn scientific_tools(
    force: bool,
    state: State<'_, AppState>,
) -> AppResult<scientific::ToolReport> {
    if force {
        let _operation = state
            .begin_operation(operation::TOOL)
            .ok_or_else(|| busy_error(&state))?;
        runtime::wsl::ensure_owned(&state.paths.app_data_dir)?;
        state.mark_runtime_owned();
        return scientific::verify_now(&state.paths.app_data_dir);
    }
    Ok(scientific::report(&state.paths.app_data_dir, false))
}

/// Installs or repairs a tool, streaming progress as `tool-progress` events.
///
/// Returns the freshly verified report rather than unit, so the caller never has to guess
/// what changed or re-query to find out.
#[tauri::command(async)]
pub fn install_scientific_tool(
    tool: scientific::Tool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<scientific::ToolReport> {
    let _operation = state
        .begin_operation(operation::TOOL)
        .ok_or_else(|| busy_error(&state))?;
    runtime::wsl::ensure_owned(&state.paths.app_data_dir)?;
    state.mark_runtime_owned();
    runtime::health::require_space(&state.paths.app_data_dir, 3)?;
    let mut emit = |progress: scientific::ToolProgress| {
        let _ = app.emit("tool-progress", &progress);
    };
    scientific::install(tool, &state.paths.app_data_dir, &mut emit)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recovery {
    Service,
    Environment,
    Verify,
    Rebuild,
    Restore,
}

#[tauri::command(async)]
pub fn recover(action: Recovery, app: AppHandle, state: State<'_, AppState>) -> AppResult<String> {
    let _operation = state
        .begin_operation(operation::RECOVERY)
        .ok_or_else(|| busy_error(&state))?;
    let paths = runtime::provision::SetupPaths {
        app_data_dir: state.paths.app_data_dir.clone(),
        workspace_dir: state.paths.workspace_dir.clone(),
    };
    let workspace = state.require_active_workspace()?;
    if matches!(action, Recovery::Verify) {
        runtime::health::fast(&workspace)?;
        runtime::provision::verify_installation()?;
        let session = state.session_for(&workspace)?;
        crate::jupyter::verify_kernels(&session)?;
        return Ok("SageMath, Python, and the notebook connection passed their checks.".into());
    }
    state.shutdown_jupyter();
    for (label, window) in app.webview_windows() {
        if label.starts_with("notebook-") {
            let _ = window.close();
        }
    }
    match action {
        Recovery::Service => {
            state.session_for(&workspace)?;
        }
        Recovery::Environment => {
            wsl::stop_checked()?;
            runtime::health::fast(&workspace)?;
            state.session_for(&workspace)?;
        }
        Recovery::Rebuild | Recovery::Restore => {
            let package = state
                .selected_package()
                .or_else(|| runtime::image::find_image(&state.paths.package_search_dirs));
            runtime::repair::replace(
                &paths,
                package.as_deref(),
                matches!(action, Recovery::Restore),
            )?;
        }
        Recovery::Verify => unreachable!(),
    }
    Ok("Your computing environment is ready. Your saved notebooks were preserved.".into())
}

#[tauri::command(async)]
pub fn diagnostic_report(state: State<'_, AppState>) -> AppResult<String> {
    if state.is_busy() {
        return Err(busy_error(&state));
    }
    // A restart setup recorded for itself is the one restart that genuinely blocks SageDock,
    // so the diagnostic must see it. Windows' own registry restart flags are not a substitute:
    // see `system::reboot`.
    let persisted = runtime::PersistedSetupState::load(&state.paths.app_data_dir);
    let checks =
        crate::system::run_system_check(persisted.awaiting_restart_now(&state.paths.app_data_dir));
    let runtime = runtime::health::fast(&state.active_workspace_dir());
    // Deliberately no raw stdout, notebook names, home paths, or server URLs in exports.
    let safe: Vec<_> = checks
        .checks
        .iter()
        .map(|c| serde_json::json!({"check":c.id,"severity":c.severity,"summary":c.summary}))
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "app":"SageDock", "version":env!("CARGO_PKG_VERSION"), "checked_at":checks.checked_at,
        "overall": checks.overall,
        "workspace_count": state.workspaces(|store| store.workspaces.len()),
        "checks":safe, "runtime":runtime.err().map(|e| e.code).unwrap_or_else(|| "healthy".into()),
        "privacy":"Notebook contents, filenames, personal paths, and authentication tokens are excluded."
    })).map_err(|e| ui_error(e.to_string()))
}

#[tauri::command(async)]
pub fn save_diagnostic_report(app: AppHandle, state: State<'_, AppState>) -> AppResult<bool> {
    let report = diagnostic_report(state)?;
    let Some(file) = app
        .dialog()
        .file()
        .set_file_name("SageDock-diagnostics.json")
        .blocking_save_file()
    else {
        return Ok(false);
    };
    let path = file.into_path().map_err(|e| ui_error(e.to_string()))?;
    crate::storage::atomic_write(&path, report.as_bytes()).map_err(|e| ui_error(e.to_string()))?;
    Ok(true)
}

#[tauri::command(async)]
pub fn restart_windows(state: State<'_, AppState>) -> AppResult<()> {
    if state.is_busy() {
        return Err(busy_error(&state));
    }
    let result = crate::system::process::run_hidden("shutdown.exe", &["/r", "/t", "0"])
        .map_err(|e| ui_error(e.to_string()))?
        .ok_or_else(|| ui_error("Windows restart unavailable"))?;
    if !result.success {
        return Err(ui_error(result.combined_output));
    }
    Ok(())
}

fn ui_error(details: impl Into<String>) -> AppError {
    AppError::new("desktop", "DESKTOP_ACTION_FAILED", "SageDock couldn't finish this action",
        "Your saved notebooks are safe. Try again. You can choose to open notebooks in your browser in Settings.")
        .with_technical_details(details)
}

#[cfg(test)]
mod tests {
    #[test]
    fn notebook_navigation_is_restricted_to_this_session() {
        for url in [
            "https://example.com",
            "http://127.0.0.1:8889",
            "http://localhost:8888",
            "file:///C:/x",
            "http://user@127.0.0.1:8888",
        ] {
            assert!(!super::allowed_navigation(&url.parse().unwrap(), 8888));
        }
        assert!(super::allowed_navigation(
            &"http://127.0.0.1:8888/lab".parse().unwrap(),
            8888
        ));
    }
}
