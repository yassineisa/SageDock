//! Tauri commands, the only bridge between the frontend and backend logic. The frontend
//! never constructs shell commands or supplies absolute file paths; it calls typed commands and
//! gets back typed results or a structured `AppError`.
//!
//! Anything that can take longer than a moment is declared `#[tauri::command(async)]`. A
//! plain synchronous command runs on the main thread, where a slow one freezes the window's
//! event loop, including delivery of the progress events meant to show it isn't frozen.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::config::{AppConfig, ThemePreference};
use crate::error::{AppError, AppResult, ErrorSeverity};
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
/// Takes an identifier from `installed_browsers`, never a path, see `browsers.rs` for why
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
/// value rather than only marking it done so Settings can offer to show it again, without
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

/// Publishes setup snapshots to the frontend as `setup-progress` events.
struct EventSink(AppHandle);

impl crate::setup::ProgressSink for EventSink {
    fn publish(&self, snapshot: &crate::setup::SetupSnapshot) {
        let _ = self.0.emit("setup-progress", snapshot);
    }
}

/// Releases the operation slot however the setup thread ends, including a panic.
///
/// The guard used elsewhere borrows `AppState`, which cannot outlive a command; this one
/// carries an `AppHandle` instead so ownership can live on the background thread, and
/// releases the slot on every return path out of the worker, including the early ones.
///
/// One honest limitation: release builds set `panic = "abort"`, so a panic inside the
/// worker takes the process down rather than unwinding, and this `Drop` never runs. That
/// is not a leak, the whole app is gone, but it does mean the guard's panic-safety only
/// applies to debug and test builds. What it always covers is the ordinary case: every
/// `return` and `?` on the way out.
struct ThreadOperation(AppHandle);

impl Drop for ThreadOperation {
    fn drop(&mut self) {
        self.0.state::<AppState>().end_operation();
    }
}

/// Starts environment setup on a background thread and returns immediately.
///
/// Returning the snapshot rather than the outcome is the point. Setup is a backend
/// operation that outlives whichever screen asked for it: the caller gets the operation id
/// and then observes progress like any other screen, by listening for `setup-progress` or
/// by calling [`setup_snapshot`]. Navigating away cannot orphan it, and a screen that
/// mounts halfway through recovers the full picture by asking.
#[tauri::command(async)]
pub fn run_setup(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<crate::setup::SetupSnapshot> {
    let paths = SetupPaths {
        app_data_dir: state.paths.app_data_dir.clone(),
        workspace_dir: state.paths.workspace_dir.clone(),
    };
    let package = current_package(&state);

    // Claimed synchronously, before returning, so a second click is refused rather than
    // racing: two overlapping setups would import into the same environment at once.
    let claimed = state.claim_operation(operation::SETUP);
    if !claimed {
        return Err(busy_error(&state));
    }

    let sink = EventSink(app.clone());
    let operation_id = state.setup.begin(Some(&sink));
    let snapshot = state.setup.snapshot();

    let worker = app.clone();
    let id = operation_id.clone();
    // `Builder::spawn` rather than `thread::spawn`, which panics when the OS refuses a
    // thread. The slot is already claimed at this point, so a panic here would strand the
    // app as permanently busy, precisely the state this change exists to make impossible.
    let spawned = std::thread::Builder::new()
        .name("sagedock-setup".into())
        .spawn(move || {
            // Dropped on every exit path out of the worker.
            let _release = ThreadOperation(worker.clone());
            let state = worker.state::<AppState>();
            let sink = EventSink(worker.clone());

            let mut emit = |progress: runtime::SetupProgress| {
                use runtime::provision::ProgressKind;
                match progress.kind {
                    // Liveness only, deliberately does not touch the progress timestamp.
                    ProgressKind::Heartbeat => state.setup.heartbeat(Some(&sink)),
                    ProgressKind::AwaitingPermission => state.setup.set_phase(
                        Some(&sink),
                        crate::setup::SetupPhase::WaitingForPermission,
                        progress
                            .detail
                            .as_deref()
                            .unwrap_or("Waiting for permission"),
                    ),
                    ProgressKind::WaitingForWindows => state.setup.set_phase(
                        Some(&sink),
                        crate::setup::SetupPhase::WaitingForWindows,
                        progress.detail.as_deref().unwrap_or("Waiting for Windows"),
                    ),
                    ProgressKind::Working => {
                        // Returning to working after a wait has to restore the phase too, or
                        // the UI would keep asking for a permission that has been granted.
                        if state.setup.snapshot().phase != crate::setup::SetupPhase::Running {
                            state.setup.set_phase(
                                Some(&sink),
                                crate::setup::SetupPhase::Running,
                                progress.detail.as_deref().unwrap_or("Working"),
                            );
                        }
                        state.setup.enter_stage(
                            Some(&sink),
                            progress.stage,
                            progress.detail.as_deref(),
                            progress.percent,
                        );
                    }
                }
            };

            let result = runtime::provision::run_setup(&paths, package.as_deref(), &id, &mut emit);

            match result {
                Ok(outcome) => {
                    if outcome == SetupOutcome::Ready {
                        // Having installed the environment, this instance may stop it.
                        state.mark_runtime_owned();
                    }
                    let phase = match outcome {
                        SetupOutcome::Ready => crate::setup::SetupPhase::Completed,
                        SetupOutcome::AwaitingRestart => crate::setup::SetupPhase::RestartRequired,
                        // Setup ran to the point of needing a file it doesn't have. That is a
                        // stop with a specific next action, not a crash, and it is reported as
                        // a failure phase so the UI offers that action rather than claiming
                        // success.
                        SetupOutcome::NeedsSagePackage => crate::setup::SetupPhase::Failed,
                    };
                    let problem = (outcome == SetupOutcome::NeedsSagePackage).then(|| {
                        AppError::new(
                            "setup",
                            "SAGE_PACKAGE_MISSING",
                            "SageDock needs the SageMath package",
                            "The SageMath package wasn't found on this PC. If you received it \
                         separately, choose it below and run setup again. A complete SageDock \
                         installer includes this file.",
                        )
                        .with_severity(ErrorSeverity::Warning)
                    });
                    state
                        .setup
                        .finish(Some(&sink), phase, Some(outcome), problem);
                    tracing::info!(target: "setup", ?outcome, "setup finished");
                }
                Err(err) => {
                    tracing::warn!(target: "setup", code = %err.code, "setup failed");
                    state.setup.finish(
                        Some(&sink),
                        crate::setup::SetupPhase::Failed,
                        None,
                        Some(err),
                    );
                }
            }
        });

    if let Err(err) = spawned {
        // Nothing is running, so the slot must go back and the operation must still reach
        // a recorded ending, an operation that simply vanished is the one outcome this
        // design does not allow.
        let problem = AppError::new(
            "setup",
            "SETUP_THREAD_FAILED",
            "SageDock couldn't start setting up",
            "Your files are safe and nothing was changed. Close some other applications to \
             free up memory, then try setup again.",
        )
        .with_technical_details(err.to_string());
        let sink = EventSink(app.clone());
        state.setup.finish(
            Some(&sink),
            crate::setup::SetupPhase::Failed,
            None,
            Some(problem.clone()),
        );
        state.end_operation();
        return Err(problem);
    }

    Ok(snapshot)
}

/// The authoritative state of setup, answerable at any moment.
///
/// This is what makes navigation safe. A screen mounting mid-operation calls this and gets
/// the whole picture, phase, stage, step list, elapsed time, log, rather than waiting for
/// the next event and showing nothing until one arrives.
#[tauri::command(async)]
pub fn setup_snapshot(state: State<'_, AppState>) -> crate::setup::SetupSnapshot {
    state.setup.snapshot()
}

/// Marks an interrupted operation as seen, so it is reported once rather than every launch.
#[tauri::command(async)]
pub fn acknowledge_setup_interruption(state: State<'_, AppState>) -> AppResult<()> {
    state.setup.clear_record()
}

/// Builds a shareable report of the setup run.
///
/// Returned as text rather than written to a file so the frontend can offer it for copying
/// without SageDock choosing a location on the user's behalf. Every line goes through the
/// same redaction as the live log: no tokens, and the Windows account name is replaced.
#[tauri::command(async)]
pub fn setup_diagnostics(state: State<'_, AppState>) -> String {
    let snapshot = state.setup.snapshot();
    let now = crate::setup::now_ms();
    let mut out = String::new();
    out.push_str("SageDock setup diagnostics\n");
    out.push_str(&format!("App version: {}\n", env!("CARGO_PKG_VERSION")));
    out.push_str(&format!("Operation: {}\n", snapshot.operation_id));
    out.push_str(&format!("Phase: {:?}\n", snapshot.phase));
    out.push_str(&format!("Stage: {:?}\n", snapshot.stage));
    match snapshot.elapsed_ms(now) {
        Some(ms) => out.push_str(&format!("Running for: {}s\n", ms / 1000)),
        None if snapshot.phase.is_terminal() => {
            out.push_str(&format!(
                "Finished after: {}s\n",
                snapshot.updated_at.saturating_sub(snapshot.started_at) / 1000,
            ));
        }
        None => out.push_str("Not started\n"),
    }
    out.push_str(&format!(
        "Last progress: {}ms before this report\n",
        now.saturating_sub(snapshot.updated_at),
    ));
    out.push_str("\nSteps\n");
    for step in &snapshot.steps {
        out.push_str(&format!("  {:?}, {}\n", step.state, step.title));
    }
    if let Some(problem) = &snapshot.problem {
        out.push_str(&format!(
            "\nProblem: {} ({})\n",
            problem.title, problem.code
        ));
        out.push_str(&format!("  {}\n", problem.message));
        if let Some(details) = &problem.technical_details {
            out.push_str(&format!("  {}\n", crate::setup::redact(details)));
        }
    }
    out.push_str("\nLog\n");
    for line in &snapshot.log {
        out.push_str(&format!(
            "  +{}s {}\n",
            line.at.saturating_sub(snapshot.started_at) / 1000,
            line.text,
        ));
    }
    // Belt and braces: the log is redacted on the way in, but the assembled report also
    // carries paths and error text from elsewhere, so the whole thing is filtered again.
    crate::setup::redact(&out)
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
