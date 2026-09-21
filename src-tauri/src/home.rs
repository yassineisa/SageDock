//! Commands behind the Home screen: the computing environment's lifecycle, the user's
//! workspaces, and portable backups of their work.
//!
//! Grouped here rather than in `commands` or `desktop` because these three share one idea —
//! they are the operations a student performs on *their own work*, as opposed to on the
//! SageMath runtime. Every one of them reports what actually happened rather than that a
//! command was issued.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

use crate::backup::{self, BackupPreview, BackupSummary, RestoreSummary};
use crate::error::{AppError, AppResult};
use crate::library;
use crate::runtime::{self, wsl};
use crate::state::{busy_error, operation, AppState};
use crate::workspaces::{self, WorkspaceView};

/// Where workspaces the user creates are kept.
///
/// A sibling of the default workspace, never a child of it: nesting would make every
/// workspace's files show up inside the default workspace's file browser and be counted
/// twice in a backup.
pub(crate) fn workspaces_parent(state: &AppState) -> PathBuf {
    match state.paths.workspace_dir.parent() {
        Some(parent) => parent.join("SageDock Workspaces"),
        None => state.paths.workspace_dir.join("Workspaces"),
    }
}

// --- environment lifecycle -------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct EnvironmentStatus {
    /// SageMath's environment is installed on this PC.
    pub installed: bool,
    /// It is running right now, holding memory.
    pub running: Option<bool>,
    /// At least one notebook service is answering.
    pub notebooks_running: bool,
    pub open_workspaces: usize,
    /// What SageDock is doing, if anything — used to disable conflicting actions.
    pub busy: Option<String>,
}

fn observe(state: &AppState) -> EnvironmentStatus {
    let installed = wsl::distro_exists(wsl::distro_name());
    EnvironmentStatus {
        installed,
        // Observed, never inferred from whether we once started it.
        running: if installed {
            wsl::distro_running_state()
        } else {
            Some(false)
        },
        notebooks_running: state.has_server(),
        open_workspaces: state.live_server_count(),
        busy: state.current_operation().map(String::from),
    }
}

#[tauri::command(async)]
pub fn environment_status(state: State<'_, AppState>) -> EnvironmentStatus {
    observe(&state)
}

/// Stops the notebook services and then SageDock's own Linux environment.
///
/// Ordered deliberately: Jupyter is asked to shut down first so kernels can exit cleanly,
/// and only then is the environment stopped underneath them. The app stays open — the next
/// notebook or workspace launch starts everything again.
#[tauri::command(async)]
pub fn stop_environment(state: State<'_, AppState>) -> AppResult<EnvironmentStatus> {
    let guard = state
        .begin_operation(operation::STOPPING)
        .ok_or_else(|| busy_error(&state))?;

    if !wsl::distro_exists(wsl::distro_name()) {
        drop(guard);
        return Ok(observe(&state));
    }

    wsl::ensure_owned(&state.paths.app_data_dir)?;
    state.shutdown_jupyter();
    // `stop_checked` terminates SageDock's distribution by name. Never `wsl --shutdown`,
    // which would stop every distribution on the machine, including ones belonging to
    // someone's unrelated work.
    wsl::stop_checked()?;

    // Report what is true, not that the command returned. A terminate that silently fails
    // to take effect would otherwise be presented as success while memory stayed held.
    if wsl::distro_running_state() != Some(false) {
        drop(guard);
        return Err(AppError::new(
            "runtime",
            "STOP_DID_NOT_TAKE_EFFECT",
            "SageMath is still running",
            "SageDock asked the computing environment to stop, but it's still running. A calculation may still be finishing. Wait a moment and try again — your saved notebooks are safe.",
        ));
    }

    tracing::info!(target: "runtime", "environment stopped on user request");
    drop(guard);
    Ok(observe(&state))
}

// --- workspaces ---------------------------------------------------------------------------

#[tauri::command(async)]
pub fn list_workspaces(state: State<'_, AppState>) -> Vec<WorkspaceView> {
    state.workspaces(|store| store.views())
}

#[tauri::command(async)]
pub fn create_workspace(name: String, state: State<'_, AppState>) -> AppResult<WorkspaceView> {
    let _guard = state
        .begin_operation(operation::WORKSPACE)
        .ok_or_else(|| busy_error(&state))?;
    let parent = workspaces_parent(&state);
    std::fs::create_dir_all(&parent).map_err(|err| {
        AppError::new(
            "workspace",
            "WORKSPACE_CREATE_FAILED",
            "SageDock couldn't create that workspace",
            "The folder that holds your workspaces couldn't be created. Check that you have free disk space and permission to save in your Documents folder.",
        )
        .with_technical_details(format!("{}: {err}", parent.display()))
    })?;

    let id = state.workspace_transaction(|store, dir| {
        let record = workspaces::create(store, &parent, &name)?;
        if let Err(err) = store.save(dir) {
            // Remove only the empty root we just created. Never recursively delete a
            // folder that the user or a sync client may already have written into.
            let _ = std::fs::remove_dir(&record.path);
            return Err(err);
        }
        Ok(record.id)
    })?;

    Ok(view_of(&state, &id))
}

/// Adds a folder the user already has, chosen in a native picker.
#[tauri::command(async)]
pub fn add_workspace_folder(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Option<WorkspaceView>> {
    let Some(picked) = app
        .dialog()
        .file()
        .set_title("Choose a folder to use as a workspace")
        .blocking_pick_folder()
    else {
        return Ok(None);
    };

    let path = picked.into_path().map_err(|err| {
        AppError::new(
            "workspace",
            "WORKSPACE_UNREADABLE",
            "SageDock couldn't use that folder",
            "SageDock wasn't able to use the folder you chose. If it's on a network location or a removable drive, copy it onto this PC first.",
        )
        .with_technical_details(err.to_string())
    })?;

    let _guard = state
        .begin_operation(operation::WORKSPACE)
        .ok_or_else(|| busy_error(&state))?;
    let id = state.update_workspaces(|store| Ok(workspaces::add_existing(store, &path)?.id))?;
    Ok(Some(view_of(&state, &id)))
}

#[tauri::command(async)]
pub fn rename_workspace(
    id: String,
    name: String,
    state: State<'_, AppState>,
) -> AppResult<WorkspaceView> {
    let _guard = state
        .begin_operation(operation::WORKSPACE)
        .ok_or_else(|| busy_error(&state))?;

    // Renaming moves the folder, which would strand a notebook server rooted inside it.
    //
    // Scoped to *this* folder. Asking whether any server was running anywhere refused every
    // rename as soon as one notebook was open in any course — including the Home launcher's
    // own session — so renaming appeared to do nothing at all.
    let path = state.workspaces(|store| store.find(&id).map(|w| w.path.clone()));
    if let Some(path) = path {
        if state.has_server_under(&path) && path.is_dir() {
            return Err(AppError::new(
                    "workspace",
                    "WORKSPACE_IN_USE",
                    "Stop SageMath before renaming a workspace",
                    "Renaming moves the folder on disk, and a notebook is still open inside it. Use Stop SageMath on the Home screen, then rename it. Nothing has been changed.",
                ));
        }
    }

    state.workspace_transaction(|store, dir| {
        let before = store
            .find(&id)
            .cloned()
            .ok_or_else(missing_workspace_error)?;
        let renamed = workspaces::rename(store, &id, &name)?;
        if let Err(err) = store.save(dir) {
            if renamed.path != before.path && renamed.path.exists() {
                std::fs::rename(&renamed.path, &before.path).map_err(|rollback| {
                    AppError::new(
                        "workspace",
                        "WORKSPACE_RENAME_RECOVERY",
                        "Your folder was renamed but the list couldn't be saved",
                        format!(
                            "Your files remain in {}. Use Add existing folder to reconnect them.",
                            renamed.path.display()
                        ),
                    )
                    .with_technical_details(format!("{err:?}; rollback: {rollback}"))
                })?;
            }
            return Err(err);
        }
        Ok(())
    })?;
    Ok(view_of(&state, &id))
}

/// Removes a workspace from the Home list. The folder and its files are left alone.
#[tauri::command(async)]
pub fn forget_workspace(id: String, state: State<'_, AppState>) -> AppResult<Vec<WorkspaceView>> {
    let _guard = state
        .begin_operation(operation::WORKSPACE)
        .ok_or_else(|| busy_error(&state))?;
    state.update_workspaces(|store| workspaces::forget(store, &id))?;
    Ok(state.workspaces(|store| store.views()))
}

/// Makes a workspace active and opens JupyterLab rooted at its folder.
#[tauri::command(async)]
pub fn launch_workspace(id: String, app: AppHandle, state: State<'_, AppState>) -> AppResult<()> {
    let _guard = state
        .begin_operation(operation::NOTEBOOK)
        .ok_or_else(|| busy_error(&state))?;

    let record = state
        .workspaces(|store| store.find(&id).cloned())
        .ok_or_else(missing_workspace_error)?;

    if !record.path.is_dir() {
        return Err(AppError::new(
            "workspace",
            "WORKSPACE_FOLDER_MISSING",
            "SageDock can't find that workspace folder",
            format!(
                "The folder for {} isn't where SageDock expects it. It may have been moved, renamed, or be on a drive that isn't connected. You can remove it from the list without deleting anything, then add it again from its new location.",
                record.name
            ),
        )
        .with_technical_details(record.path.display().to_string()));
    }

    runtime::health::fast(&record.path)?;
    runtime::workspace::ensure_workspace(&record.path)?;

    let session = state.session_for(&record.path)?;
    // The folder itself, not the launcher: a workspace card opens that course's files.
    crate::desktop::open_in_viewer(
        &app,
        &state,
        &session,
        crate::desktop::ViewerTarget::Path(""),
    )?;
    state.update_workspaces(|store| {
        workspaces::set_active(store, &id)?;
        workspaces::touch(store, &id);
        Ok(())
    })?;

    // A server already rooted here is reused, so returning to a workspace rejoins any
    // kernels still computing in it rather than restarting them.
    Ok(())
}

/// Opens a workspace folder in File Explorer.
#[tauri::command(async)]
pub fn reveal_workspace(id: String, state: State<'_, AppState>) -> AppResult<()> {
    let record = state
        .workspaces(|store| store.find(&id).cloned())
        .ok_or_else(missing_workspace_error)?;

    if !record.path.is_dir() {
        return Err(AppError::new(
            "workspace",
            "WORKSPACE_FOLDER_MISSING",
            "SageDock can't find that workspace folder",
            "The folder isn't where SageDock expects it. It may have been moved, renamed, or be on a drive that isn't connected.",
        ));
    }

    // Passed as a single argument, never through a shell.
    std::process::Command::new("explorer.exe")
        .arg(&record.path)
        .spawn()
        .map_err(|err| {
            AppError::new(
                "workspace",
                "OPEN_FOLDER_FAILED",
                "SageDock couldn't open that folder",
                "SageDock wasn't able to open the folder in File Explorer.",
            )
            .with_technical_details(err.to_string())
        })?;
    Ok(())
}

/// Adds files the user picked into a workspace folder. Nothing is ever overwritten.
#[tauri::command(async)]
pub fn add_files_to_workspace(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<usize> {
    let record = state
        .workspaces(|store| store.find(&id).cloned())
        .ok_or_else(missing_workspace_error)?;

    if !record.path.is_dir() {
        return Err(AppError::new(
            "workspace",
            "WORKSPACE_FOLDER_MISSING",
            "SageDock can't find that workspace folder",
            "The folder isn't where SageDock expects it, so there's nowhere to put the files. Nothing has been copied.",
        ));
    }

    // The picker runs before the operation slot is claimed: it blocks until the user
    // chooses, and holding the lock across that would make the whole app look stuck.
    let Some(picked) = app
        .dialog()
        .file()
        .set_title("Choose files to add to this workspace")
        .blocking_pick_files()
    else {
        return Ok(0);
    };

    let _guard = state
        .begin_operation(operation::WORKSPACE)
        .ok_or_else(|| busy_error(&state))?;

    let mut added = 0;
    for file in picked {
        let path = file.into_path().map_err(|err| {
            AppError::new(
                "workspace",
                "WORKSPACE_FILE_UNREADABLE",
                "SageDock couldn't read one of those files",
                "If the file is on a network location or a removable drive, copy it onto this PC first. Any files already added have been kept.",
            )
            .with_technical_details(err.to_string())
        })?;
        crate::library::add_file(&record.path, &path)?;
        added += 1;
    }

    tracing::info!(target: "workspace", added, "files added to a workspace");
    Ok(added)
}

/// What a drop onto the window did.
///
/// Reported as an event rather than returned, because a drop has no caller to return to.
#[derive(Clone, Serialize)]
pub struct DropOutcome {
    pub workspace: String,
    pub added: usize,
    pub failed: usize,
    /// Set when nothing could be copied at all, phrased for the user.
    pub error: Option<String>,
}

/// Copies files dropped on the window into the workspace the student is working in.
///
/// Called from the window event handler rather than from a command, deliberately: the
/// absolute paths Windows reports for a drop stay in the backend and are never handed to
/// the webview, which is the same rule every other file operation here follows.
pub fn drop_files(state: &AppState, paths: &[PathBuf]) -> DropOutcome {
    let workspace_name = state
        .workspaces(|store| store.active_record().map(|record| record.name.clone()))
        .unwrap_or_default();
    let refused = |message: String| DropOutcome {
        workspace: workspace_name.clone(),
        added: 0,
        failed: paths.len(),
        error: Some(message),
    };

    let workspace = match state.require_active_workspace() {
        Ok(path) => path,
        Err(err) => return refused(err.message),
    };
    // A drop during setup or a restore must not write into a folder being rebuilt.
    let Some(_guard) = state.begin_operation(operation::WORKSPACE) else {
        return refused(busy_error(state).message);
    };

    let mut added = 0;
    let mut failed = 0;
    for path in paths {
        match crate::library::add_file(&workspace, path) {
            Ok(_) => added += 1,
            Err(err) => {
                failed += 1;
                tracing::warn!(target: "workspace", code = %err.code, "a dropped file could not be added");
            }
        }
    }

    tracing::info!(target: "workspace", added, failed, "files dropped onto the window");
    DropOutcome {
        workspace: workspace_name,
        added,
        failed,
        error: None,
    }
}

// --- downloaded notebooks ----------------------------------------------------------------

/// What the student chose to do about a name already in use in the target workspace.
#[derive(serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum DownloadConflict {
    /// Open what is already there and copy nothing.
    OpenExisting,
    /// Overwrite the existing file with the download.
    Replace,
    /// Keep both, giving the download a new, uncolliding name.
    Copy,
}

/// What choosing a workspace for a downloaded notebook would do, before doing it.
#[derive(Serialize)]
pub struct DownloadTarget {
    pub workspace_name: String,
    /// The file name the notebook would land as; see `library::target_notebook_name`.
    pub target_name: String,
    /// Whether that name is already taken in the chosen workspace.
    pub exists: bool,
}

fn workspace_record(
    state: &AppState,
    workspace_id: &str,
) -> AppResult<crate::workspaces::WorkspaceRecord> {
    let record = state
        .workspaces(|store| store.find(workspace_id).cloned())
        .ok_or_else(missing_workspace_error)?;
    if !record.path.is_dir() {
        return Err(AppError::new(
            "workspace",
            "WORKSPACE_FOLDER_MISSING",
            "SageDock can't find that workspace folder",
            format!(
                "The folder for {} isn't where SageDock expects it. It may have been moved, renamed, or be on a drive that isn't connected.",
                record.name
            ),
        )
        .with_technical_details(record.path.display().to_string()));
    }
    Ok(record)
}

fn no_pending_import_error() -> AppError {
    AppError::new(
        "notebook",
        "NO_PENDING_IMPORT",
        "SageDock lost track of that file",
        "Choose the notebook again from Open notebook.",
    )
}

/// What choosing a workspace for `source` would do, without doing it. Shared by the
/// Downloads flow and the "Open notebook" picker flow, so both agree on what counts as a
/// collision.
fn check_target(state: &AppState, source: &Path, workspace_id: &str) -> AppResult<DownloadTarget> {
    let record = workspace_record(state, workspace_id)?;
    let file_name = source
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported Notebook.ipynb");
    let target_name = library::target_notebook_name(file_name);
    let exists = record.path.join(&target_name).is_file();
    Ok(DownloadTarget {
        workspace_name: record.name,
        target_name,
        exists,
    })
}

/// Copies `source` into the chosen workspace and opens it there. Shared by the Downloads
/// flow and the "Open notebook" picker flow — both end the same way once a workspace and a
/// conflict decision are known.
///
/// The target need not be the workspace that was active a moment ago — a student can send a
/// notebook to any course folder, not only the one Home happened to be showing. Opening it
/// afterwards makes that workspace active, the same as clicking Launch workspace on its
/// card, so returning to Home reflects where the notebook actually landed.
fn finish_import(
    app: &AppHandle,
    state: &AppState,
    source: &Path,
    workspace_id: &str,
    on_conflict: DownloadConflict,
) -> AppResult<String> {
    let record = workspace_record(state, workspace_id)?;
    runtime::workspace::ensure_workspace(&record.path)?;

    let file_name = source
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported Notebook.ipynb");
    let relative = match on_conflict {
        DownloadConflict::Copy => library::import(&record.path, source)?,
        DownloadConflict::Replace => library::import_replacing(&record.path, source)?,
        DownloadConflict::OpenExisting => {
            let target = library::target_notebook_name(file_name);
            if record.path.join(&target).is_file() {
                target
            } else {
                // The file that was there when the student was asked is gone by the time
                // they answered. Copying it in is the honest fallback: there is nothing
                // left to open, and refusing outright would strand the notebook.
                library::import(&record.path, source)?
            }
        }
    };

    runtime::health::fast(&record.path)?;
    let session = state.session_for(&record.path)?;
    crate::desktop::open_in_viewer(
        app,
        state,
        &session,
        crate::desktop::ViewerTarget::Path(&relative),
    )?;

    state.update_workspaces(|store| {
        workspaces::set_active(store, workspace_id)?;
        workspaces::touch(store, workspace_id);
        Ok(())
    })?;

    Ok(relative)
}

/// Checks what choosing a workspace for a downloaded notebook would do, without doing it.
///
/// Read-only and side-effect free by design: this runs the moment a student picks a
/// workspace in the dialog, purely to decide whether to ask about a name collision before
/// anything is copied.
#[tauri::command(async)]
pub fn check_download_target(
    name: String,
    workspace_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<DownloadTarget> {
    // Confirms the file is still there and still a legitimately named download before the
    // student is asked to make a decision about it. Accepts either kind of key, so a
    // notebook the built-in browser saved outside Downloads behaves like any other.
    let source = library::resolve_download(
        &crate::desktop::downloads_dir(&app).unwrap_or_default(),
        &state.tracked_downloads(),
        &name,
    )?;
    check_target(&state, &source, &workspace_id)
}

/// Copies a downloaded notebook into the chosen workspace and opens it there.
#[tauri::command(async)]
pub fn open_downloaded_notebook(
    name: String,
    workspace_id: String,
    on_conflict: DownloadConflict,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let _guard = state
        .begin_operation(operation::NOTEBOOK)
        .ok_or_else(|| busy_error(&state))?;
    let source = library::resolve_download(
        &crate::desktop::downloads_dir(&app).unwrap_or_default(),
        &state.tracked_downloads(),
        &name,
    )?;
    finish_import(&app, &state, &source, &workspace_id, on_conflict)
}

/// Checks what choosing a workspace for the notebook picked via "Open notebook" would do.
#[tauri::command(async)]
pub fn check_picked_target(
    workspace_id: String,
    state: State<'_, AppState>,
) -> AppResult<DownloadTarget> {
    let source = state
        .selected_import()
        .ok_or_else(no_pending_import_error)?;
    check_target(&state, &source, &workspace_id)
}

/// Copies the notebook picked via "Open notebook" into the chosen workspace and opens it.
///
/// Consumes the pending choice, so answering the dialog a second time (a stray double
/// click, a repeated call) reports the same clear error rather than silently reimporting an
/// already-handled file.
#[tauri::command(async)]
pub fn open_picked_notebook(
    workspace_id: String,
    on_conflict: DownloadConflict,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let _guard = state
        .begin_operation(operation::NOTEBOOK)
        .ok_or_else(|| busy_error(&state))?;
    let source = state
        .take_selected_import()
        .ok_or_else(no_pending_import_error)?;
    finish_import(&app, &state, &source, &workspace_id, on_conflict)
}

fn view_of(state: &AppState, id: &str) -> WorkspaceView {
    state.workspaces(|store| {
        store
            .views()
            .into_iter()
            .find(|view| view.id == id)
            .unwrap_or_else(|| WorkspaceView {
                id: id.to_string(),
                name: String::new(),
                path: String::new(),
                last_opened: None,
                is_active: false,
                status: "missing",
            })
    })
}

fn missing_workspace_error() -> AppError {
    AppError::new(
        "workspace",
        "WORKSPACE_UNKNOWN",
        "SageDock couldn't find that workspace",
        "That workspace is no longer on your list. Return to the Home screen to see your current workspaces. Your files are not affected.",
    )
}

// --- backups -------------------------------------------------------------------------------

#[derive(Serialize)]
pub struct ChosenBackup {
    pub file_name: String,
    pub preview: BackupPreview,
}

/// Creates a verified, portable backup of every workspace.
#[tauri::command(async)]
pub fn create_backup(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Option<BackupSummary>> {
    // Built from the date parts rather than a format description, which would need the
    // `time` crate's macros feature for what is one filename.
    let now = time::OffsetDateTime::now_utc();
    let suggested = format!(
        "SageDock-backup-{:04}-{:02}-{:02}.zip",
        now.year(),
        u8::from(now.month()),
        now.day()
    );

    let Some(picked) = app
        .dialog()
        .file()
        .set_title("Save your SageDock backup")
        .set_file_name(&suggested)
        .add_filter("SageDock backup", &["zip"])
        .blocking_save_file()
    else {
        return Ok(None);
    };

    let destination = picked.into_path().map_err(|err| {
        AppError::new(
            "backup",
            "BACKUP_DESTINATION_UNUSABLE",
            "SageDock couldn't save to that location",
            "Choose somewhere else to save your backup, such as your Desktop or a USB drive.",
        )
        .with_technical_details(err.to_string())
    })?;

    let _guard = state
        .begin_operation(operation::BACKUP)
        .ok_or_else(|| busy_error(&state))?;

    let mut emit = |progress: backup::BackupProgress| {
        let _ = app.emit("backup-progress", &progress);
    };

    let config = state.get_config();
    let summary =
        state.workspaces(|store| backup::create(store, &config, &destination, &mut emit))?;

    tracing::info!(target: "backup", files = summary.file_count, "backup created");
    Ok(Some(summary))
}

/// Opens a backup and describes what restoring it would do. Changes nothing.
#[tauri::command(async)]
pub fn preview_backup(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<Option<ChosenBackup>> {
    let Some(picked) = app
        .dialog()
        .file()
        .set_title("Choose a SageDock backup")
        .add_filter("SageDock backup", &["zip"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };

    let path = picked.into_path().map_err(|err| {
        AppError::new(
            "backup",
            "BACKUP_UNREADABLE",
            "SageDock couldn't open that backup",
            "SageDock wasn't able to read the file you chose. If it's on a USB drive or a network location, copy it onto this PC first.",
        )
        .with_technical_details(err.to_string())
    })?;

    let parent = workspaces_parent(&state);
    let preview = state.workspaces(|store| backup::preview_in(&path, store, Some(&parent)))?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    // Remembered so restoring needs no path from the frontend.
    state.set_selected_backup(Some(path));
    Ok(Some(ChosenBackup { file_name, preview }))
}

/// Restores the previewed backup, always alongside existing work and never over it.
#[tauri::command(async)]
pub fn restore_backup(app: AppHandle, state: State<'_, AppState>) -> AppResult<RestoreSummary> {
    let path = state.selected_backup().ok_or_else(|| {
        AppError::new(
            "backup",
            "BACKUP_NOT_CHOSEN",
            "Choose a backup first",
            "Use Restore backup to pick the backup file you'd like to restore from.",
        )
    })?;

    let _guard = state
        .begin_operation(operation::RESTORE)
        .ok_or_else(|| busy_error(&state))?;

    let parent = workspaces_parent(&state);
    std::fs::create_dir_all(&parent).map_err(|err| {
        AppError::new(
            "backup",
            "RESTORE_FAILED",
            "SageDock couldn't prepare a place for the restored work",
            "The folder that holds your workspaces couldn't be created. Check available disk space and try again.",
        )
        .with_technical_details(format!("{}: {err}", parent.display()))
    })?;

    let mut emit = |progress: backup::BackupProgress| {
        let _ = app.emit("backup-progress", &progress);
    };

    let summary = state.workspace_transaction(|store, dir| {
        backup::restore_with_commit(&path, &parent, store, &mut emit, &mut |updated| {
            updated.save(dir)
        })
    })?;

    state.set_selected_backup(None);
    tracing::info!(target: "backup", restored = summary.restored.len(), "backup restored");
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Workspaces must sit beside the default one, not inside it — nesting would duplicate
    /// every file in a backup and show workspaces inside each other's file browsers.
    #[test]
    fn managed_workspaces_are_siblings_of_the_default_workspace() {
        let default = std::path::Path::new(r"C:\Users\someone\Documents\SageDock");
        let parent = match default.parent() {
            Some(parent) => parent.join("SageDock Workspaces"),
            None => default.join("Workspaces"),
        };

        assert_eq!(
            parent,
            PathBuf::from(r"C:\Users\someone\Documents\SageDock Workspaces")
        );
        assert!(
            !parent.starts_with(default),
            "must not nest inside the default workspace"
        );
    }
}
