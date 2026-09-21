//! Tauri commands — the only bridge between the frontend and backend logic. The frontend
//! never constructs shell commands or supplies absolute file paths; it calls typed commands and
//! gets back typed results or a structured `AppError`.
//!
//! Anything that can take longer than a moment is declared `#[tauri::command(async)]`. A
//! plain synchronous command runs on the main thread, where a slow one freezes the window's
//! event loop — including delivery of the progress events meant to show it isn't frozen.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::config::{AppConfig, ThemePreference};
use crate::error::{AppError, AppResult};
use crate::jupyter::{self, notebook::NotebookKind};
use crate::runtime::{self, image, provision::SetupPaths, wsl, PersistedSetupState, SetupOutcome};
use crate::state::{busy_error, operation, AppState};

#[derive(Clone, Serialize)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
    /// Shown in the About section. The app is MIT-licensed; the bundled scientific runtime
    /// is not, and says so there.
    pub developer: &'static str,
    pub license: &'static str,
}

#[tauri::command]
pub fn get_app_info() -> AppInfo {
    AppInfo {
        name: "SageDock",
        version: env!("CARGO_PKG_VERSION"),
        developer: "Yassin Eisa",
        license: "MIT",
    }
}

/// The developer's source page, linked from About.
const PROJECT_URL: &str = "https://github.com/yassineisa";

/// Opens the project page in the user's default browser.
///
/// The address is fixed here rather than passed in: the webview cannot ask SageDock to open
/// an arbitrary URL, so this stays a single named operation instead of a general-purpose
/// link opener. Opening the browser can block, hence `async`.
#[tauri::command(async)]
pub fn open_project_page(app: AppHandle) -> AppResult<()> {
    app.opener()
        .open_url(PROJECT_URL, None::<&str>)
        .map_err(|err| {
            AppError::new(
                "launcher",
                "OPEN_LINK_FAILED",
                "SageDock couldn't open your browser",
                "SageDock wasn't able to open the project page. You can visit github.com/yassineisa in your browser instead.",
            )
            .with_technical_details(err.to_string())
        })
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> AppConfig {
    state.get_config()
}

#[tauri::command]
pub fn set_theme(theme: ThemePreference, state: State<'_, AppState>) -> AppResult<AppConfig> {
    state.update_config(|config| config.theme = theme)
}

/// The browsers installed on this PC, for the first-run choice and the Settings picker.
///
/// Reading the registry is cheap but not instant, and this runs while a screen is being
/// drawn, hence `async` so it cannot stall the window's event loop.
#[tauri::command(async)]
pub fn installed_browsers() -> Vec<crate::browsers::Browser> {
    crate::browsers::installed()
}

/// Records which installed browser to open notebooks in. `None` means the Windows default.
///
/// Takes an identifier from `installed_browsers`, never a path — see `browsers.rs` for why
/// that distinction is load-bearing.
#[tauri::command]
pub fn set_preferred_browser(
    value: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<AppConfig> {
    state.update_config(|config| config.preferred_browser = value)
}

/// Records whether the first-run introduction has been seen.
///
/// Separate from the settings the introduction collects: skipping it must still count as
/// having seen it, or a student who skips would meet it again on every launch. Takes a
/// value rather than only marking it done so Settings can offer to show it again — without
/// that, skipping it once would put it permanently out of reach.
#[tauri::command]
pub fn set_onboarding_complete(value: bool, state: State<'_, AppState>) -> AppResult<AppConfig> {
    state.update_config(|config| config.onboarding_complete = value)
}

// --- SageMath package ---------------------------------------------------------------------

#[derive(Serialize)]
pub struct SagePackageInfo {
    pub file_name: String,
    pub folder: String,
    pub sage_version: Option<String>,
    /// Whether a checksum manifest was found, i.e. whether setup can prove the copy is intact.
    pub has_checksum: bool,
}

fn describe(package: &image::RuntimeImage) -> SagePackageInfo {
    SagePackageInfo {
        file_name: package.file_name(),
        folder: package
            .archive
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        sage_version: package.manifest.as_ref().map(|m| m.sage_version.clone()),
        has_checksum: package.manifest.is_some(),
    }
}

/// The package setup would use right now: a hand-picked one, else one found automatically.
fn current_package(state: &AppState) -> Option<PathBuf> {
    state
        .selected_package()
        .or_else(|| image::find_image(&state.paths.package_search_dirs))
}

/// Opens a native file picker for the SageMath package and validates the choice.
///
/// The dialog is opened here in the backend, not in the webview, so the frontend never
/// supplies a filesystem path of its own. Returns `None` if the user closed the picker.
#[tauri::command(async)]
pub fn choose_sage_package(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Option<SagePackageInfo>> {
    let Some(picked) = app
        .dialog()
        .file()
        .set_title("Choose the SageMath package")
        .add_filter("SageMath package for SageDock", &["xz", "gz", "tgz", "tar"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };

    let path = picked.into_path().map_err(|err| {
        AppError::new(
            "runtime",
            "SAGE_PACKAGE_UNREADABLE",
            "SageDock couldn't open the SageMath package",
            "SageDock wasn't able to use the file you chose. Try copying it onto this PC first.",
        )
        .with_technical_details(err.to_string())
    })?;

    let package = image::inspect(&path)?;
    tracing::info!(target: "setup", file = %package.file_name(), "SageMath package chosen");
    state.set_selected_package(path);
    Ok(Some(describe(&package)))
}

// --- setup ----------------------------------------------------------------------------------

#[derive(Serialize)]
pub struct SetupStatus {
    pub environment_ready: bool,
    /// SageMath is installed, even if setup hasn't finished verifying it.
    pub environment_installed: bool,
    pub awaiting_restart: bool,
    pub workspace_path: String,
    /// The package setup would install from, if SageMath still needs installing.
    pub sage_package: Option<SagePackageInfo>,
    pub problem: Option<AppError>,
    pub busy: bool,
}

#[tauri::command(async)]
pub fn get_setup_status(state: State<'_, AppState>) -> AppResult<SetupStatus> {
    setup_status(&state)
}

pub(crate) fn setup_status(state: &AppState) -> AppResult<SetupStatus> {
    let persisted = PersistedSetupState::load(&state.paths.app_data_dir);
    let installed = wsl::distro_exists(wsl::distro_name());
    let workspace = state.active_workspace_dir();

    let sage_package = if installed {
        None
    } else {
        current_package(state)
            .and_then(|path| image::inspect(&path).ok())
            .map(|package| describe(&package))
    };

    // Observe an already-running environment without waking a stopped one. Launch still
    // performs a health check before opening notebooks. A disconnected course folder must
    // not disable launching another course from Home.
    let problem = if persisted.completed && installed {
        if let Some(_guard) = state.begin_operation("checking SageMath") {
            if workspace.is_dir() && wsl::distro_is_running() {
                state
                    .require_active_workspace()
                    .and_then(|path| runtime::health::fast(&path))
                    .err()
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    Ok(SetupStatus {
        environment_ready: persisted.completed
            && installed
            && problem.is_none()
            && !state.is_busy(),
        environment_installed: installed,
        awaiting_restart: persisted.awaiting_restart_now(&state.paths.app_data_dir),
        workspace_path: workspace.display().to_string(),
        sage_package,
        problem,
        busy: state.is_busy(),
    })
}

/// Runs environment setup, streaming progress to the UI as `setup-progress` events.
#[tauri::command(async)]
pub fn run_setup(app: AppHandle, state: State<'_, AppState>) -> AppResult<SetupOutcome> {
    let paths = SetupPaths {
        app_data_dir: state.paths.app_data_dir.clone(),
        workspace_dir: state.paths.workspace_dir.clone(),
    };
    let package = current_package(&state);

    // Held for the whole run: two overlapping setups would import into the same environment
    // at once, and shutdown must not tear it down while this is in flight.
    let _guard = state
        .begin_operation(operation::SETUP)
        .ok_or_else(|| busy_error(&state))?;

    let mut emit = |progress: runtime::SetupProgress| {
        let _ = app.emit("setup-progress", &progress);
    };

    let outcome = runtime::provision::run_setup(&paths, package.as_deref(), &mut emit)?;

    // Having installed the environment, this instance is the one entitled to stop it.
    if outcome == SetupOutcome::Ready {
        state.mark_runtime_owned();
    }

    tracing::info!(target: "setup", ?outcome, "setup finished");
    Ok(outcome)
}

// --- notebooks -------------------------------------------------------------------------------

#[derive(Serialize)]
pub struct NotebookLaunch {
    pub relative_path: String,
}

/// Creates an empty notebook in the active workspace.
///
/// Deliberately does not start the notebook service: the caller opens the result, and
/// opening is what starts a server. Starting one here too would boot a second server for
/// the same folder and slow every notebook creation by the readiness wait.
#[tauri::command(async)]
pub fn new_notebook(kind: NotebookKind, state: State<'_, AppState>) -> AppResult<NotebookLaunch> {
    let _guard = state
        .begin_operation(operation::NOTEBOOK)
        .ok_or_else(|| busy_error(&state))?;

    let workspace = state.require_active_workspace()?;
    // Reports a missing environment, an unwritable workspace, or a missing kernel with
    // advice specific to each, so this never fails with a bare file error.
    runtime::health::fast(&workspace)?;

    let relative_path = jupyter::notebook::create_notebook(&workspace, kind)?;
    Ok(NotebookLaunch { relative_path })
}

/// Stops the notebook services, shuts down the Linux environment, and closes SageDock.
///
/// Ordered deliberately: Jupyter is asked to stop first so it can flush and exit cleanly,
/// and only then is the environment torn out from under it.
#[tauri::command(async)]
pub fn shutdown_and_quit(app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    // Refused rather than queued: stopping the environment during an import or a restore
    // would leave a half-written one behind, and the user can close the window again in a
    // moment. The message names the actual operation.
    if let Some(doing) = state.current_operation() {
        return Err(AppError::new(
            "launcher",
            "OPERATION_IN_PROGRESS",
            "SageDock is still working",
            format!("SageDock is still {doing}. Closing now would leave it unfinished, so please let it finish first."),
        ));
    }

    tracing::info!(target: "launcher", "shutting down on user request");
    state.shutdown_jupyter();
    wsl::terminate_distro();
    app.exit(0);
    Ok(())
}

/// Opens the folder that holds the student's workspaces in File Explorer.
///
/// Deliberately the workspaces folder rather than the active workspace. Settings describes
/// the installation as a whole, and opening whichever course happened to be selected last
/// made that row mean something different depending on what the student did before opening
/// it. The folder is created if it doesn't exist yet, so a fresh installation that has
/// never added a second workspace still opens something rather than reporting an error.
#[tauri::command(async)]
pub fn open_workspaces_folder(state: State<'_, AppState>) -> AppResult<()> {
    let path = crate::home::workspaces_parent(&state);
    std::fs::create_dir_all(&path).map_err(|err| {
        AppError::new(
            "workspace",
            "OPEN_FOLDER_FAILED",
            "SageDock couldn't open your workspaces folder",
            "SageDock wasn't able to create the folder that holds your workspaces. Check that you have free disk space and permission to save in your Documents folder.",
        )
        .with_technical_details(err.to_string())
    })?;

    // Passed as a single argument, never through a shell.
    std::process::Command::new("explorer.exe")
        .arg(&path)
        .spawn()
        .map_err(|err| {
            AppError::new(
                "workspace",
                "OPEN_FOLDER_FAILED",
                "SageDock couldn't open your folder",
                "SageDock wasn't able to open your workspaces folder in File Explorer.",
            )
            .with_technical_details(err.to_string())
        })?;

    Ok(())
}
