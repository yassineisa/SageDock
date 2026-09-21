//! In-memory application state, managed by Tauri and shared across commands.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::config::{AppConfig, ConfigStore};
use crate::downloads::DownloadStore;
use crate::error::{AppError, AppResult};
use crate::jupyter::{JupyterSession, RunningServer};
use crate::workspaces::{self, WorkspaceStore};

/// Upper bound on simultaneously running notebook servers.
///
/// Each open workspace holds its own server so switching between them never interrupts a
/// running calculation, but each one is also a real process holding real memory. Refusing
/// the next one with an explanation is kinder than silently stopping somebody's work to
/// make room, and kinder than letting the machine grind to a halt.
const MAX_LIVE_SERVERS: usize = 6;

pub struct AppState {
    config_store: ConfigStore,
    config: Mutex<AppConfig>,
    /// Running Jupyter servers, at most one per workspace folder. Held here so the process
    /// handles outlive the commands that started them and can be shut down cleanly.
    jupyter: Mutex<Vec<RunningServer>>,
    /// A SageMath package the user picked by hand. Takes precedence over auto-detection.
    selected_package: Mutex<Option<PathBuf>>,
    /// A backup file the user chose in a native picker, held between previewing it and
    /// deciding to restore it. Kept here rather than handed to the frontend and passed
    /// back, so no filesystem path ever originates in the webview.
    selected_backup: Mutex<Option<PathBuf>>,
    /// A notebook chosen via the "Open notebook" picker, held between choosing it and
    /// choosing which workspace it belongs in. Same reasoning as `selected_backup`: the
    /// absolute path is never handed to the webview and back.
    selected_import: Mutex<Option<PathBuf>>,
    /// The long-running operation in flight, described the way a person would say it.
    ///
    /// A plain "busy" flag was not enough: the same lock guards setup, repair, backup, and
    /// restore, and telling somebody "SageDock is in the middle of installing SageMath"
    /// while it is actually writing a backup is a lie the UI then acts on.
    operation: Mutex<Option<&'static str>>,
    /// True once this instance has started the Linux environment itself.
    runtime_owned: AtomicBool,
    workspaces: Mutex<WorkspaceStore>,
    /// Files the built-in browser downloaded, so Home can list one saved outside the
    /// Windows Downloads folder. See `downloads.rs` for why this is a record and not a scan.
    downloads: Mutex<DownloadStore>,
    pub paths: AppPaths,
}

/// Held for the duration of a long operation; clears the label when dropped, so an early
/// return or a panic can't leave SageDock permanently marked as busy.
pub struct OperationGuard<'a> {
    state: &'a AppState,
}

impl Drop for OperationGuard<'_> {
    fn drop(&mut self) {
        *self
            .state
            .operation
            .lock()
            .expect("operation mutex poisoned") = None;
    }
}

/// Resolved once at startup. Every path the app uses comes from here rather than being
/// rebuilt ad hoc, so nothing hardcodes a drive letter or assumes where Documents lives.
pub struct AppPaths {
    pub app_data_dir: PathBuf,
    /// The default workspace, created by setup. Workspaces the user adds later live
    /// wherever they choose; this one is only the starting point and the fallback.
    pub workspace_dir: PathBuf,
    /// Where to look for a SageMath package, most preferred first.
    pub package_search_dirs: Vec<PathBuf>,
}

impl AppState {
    pub fn new(config_store: ConfigStore, paths: AppPaths) -> Self {
        let config = config_store.load();
        let mut workspaces = WorkspaceStore::load(&paths.app_data_dir);
        // Migrates an installation that predates workspaces, and repairs an emptied list.
        if workspaces.ensure_default(&paths.workspace_dir) {
            if let Err(err) = workspaces.save(&paths.app_data_dir) {
                tracing::warn!(target: "workspace", code = %err.code, "could not record the default workspace");
            }
        }
        Self {
            config_store,
            config: Mutex::new(config),
            jupyter: Mutex::new(Vec::new()),
            selected_package: Mutex::new(None),
            selected_backup: Mutex::new(None),
            selected_import: Mutex::new(None),
            operation: Mutex::new(None),
            runtime_owned: AtomicBool::new(false),
            workspaces: Mutex::new(workspaces),
            downloads: Mutex::new(DownloadStore::load(&paths.app_data_dir)),
            paths,
        }
    }

    // --- long operations ----------------------------------------------------------------

    /// Claims the operation slot. `None` means something else is already running.
    ///
    /// `label` completes the sentence "SageDock is still …", so it reads as plain English
    /// wherever it surfaces.
    pub fn begin_operation(&self, label: &'static str) -> Option<OperationGuard<'_>> {
        let mut guard = self.operation.lock().expect("operation mutex poisoned");
        if guard.is_some() {
            return None;
        }
        *guard = Some(label);
        Some(OperationGuard { state: self })
    }

    /// What SageDock is doing right now, if anything.
    pub fn current_operation(&self) -> Option<&'static str> {
        *self.operation.lock().expect("operation mutex poisoned")
    }

    pub fn is_busy(&self) -> bool {
        self.current_operation().is_some()
    }

    /// Records that this instance started the Linux environment.
    pub fn mark_runtime_owned(&self) {
        self.runtime_owned.store(true, Ordering::SeqCst);
    }

    /// Whether this instance may stop the shared Linux environment on the way out.
    ///
    /// The environment is registered machine-wide under one name, so it is emphatically not
    /// this window's private property: an instance that never started it must not stop it,
    /// or closing an idle window would kill another window's running computations and
    /// prevent open notebooks from saving. Any operation in flight blocks it for the same
    /// reason.
    pub fn may_stop_runtime(&self) -> bool {
        self.runtime_owned.load(Ordering::SeqCst) && !self.is_busy()
    }

    // --- configuration -------------------------------------------------------------------

    pub fn get_config(&self) -> AppConfig {
        self.config.lock().expect("config mutex poisoned").clone()
    }

    pub fn update_config(&self, mutate: impl FnOnce(&mut AppConfig)) -> AppResult<AppConfig> {
        let mut guard = self.config.lock().expect("config mutex poisoned");
        let mut updated = guard.clone();
        mutate(&mut updated);
        self.config_store.save(&updated)?;
        *guard = updated.clone();
        Ok(updated)
    }

    // --- downloads from the built-in browser -------------------------------------------

    /// Remembers a file the built-in browser just downloaded.
    ///
    /// A failure to persist is logged rather than surfaced: the download itself succeeded
    /// and the file is where the student put it, so interrupting them with an error over a
    /// missing list entry would be out of proportion to what went wrong.
    pub fn record_download(&self, path: PathBuf) {
        let mut guard = self.downloads.lock().expect("downloads mutex poisoned");
        guard.record(path);
        if let Err(err) = guard.save(&self.paths.app_data_dir) {
            tracing::warn!(target: "notebook", code = %err.code, "could not record the download");
        }
    }

    /// Remembered downloads that are still on disk, most recent first.
    pub fn tracked_downloads(&self) -> Vec<PathBuf> {
        self.downloads
            .lock()
            .expect("downloads mutex poisoned")
            .existing()
    }

    /// Stops listing a download on Home. Never deletes the file; the caller does that
    /// separately when the student asked for it.
    pub fn forget_download(&self, path: &Path) {
        let mut guard = self.downloads.lock().expect("downloads mutex poisoned");
        if guard.forget(path) {
            if let Err(err) = guard.save(&self.paths.app_data_dir) {
                tracing::warn!(target: "notebook", code = %err.code, "could not update the download list");
            }
        }
    }

    pub fn selected_package(&self) -> Option<PathBuf> {
        self.selected_package
            .lock()
            .expect("package mutex poisoned")
            .clone()
    }

    pub fn set_selected_package(&self, path: PathBuf) {
        *self
            .selected_package
            .lock()
            .expect("package mutex poisoned") = Some(path);
    }

    pub fn selected_backup(&self) -> Option<PathBuf> {
        self.selected_backup
            .lock()
            .expect("backup mutex poisoned")
            .clone()
    }

    pub fn set_selected_backup(&self, path: Option<PathBuf>) {
        *self.selected_backup.lock().expect("backup mutex poisoned") = path;
    }

    /// Reads the pending import without consuming it, for the conflict check that runs
    /// before the student has committed to a workspace.
    pub fn selected_import(&self) -> Option<PathBuf> {
        self.selected_import
            .lock()
            .expect("import mutex poisoned")
            .clone()
    }

    pub fn set_selected_import(&self, path: PathBuf) {
        *self.selected_import.lock().expect("import mutex poisoned") = Some(path);
    }

    /// Consumes the pending import, so a second attempt after a finished one starts clean
    /// rather than silently reusing a file the student picked minutes earlier.
    pub fn take_selected_import(&self) -> Option<PathBuf> {
        self.selected_import
            .lock()
            .expect("import mutex poisoned")
            .take()
    }

    // --- workspaces ----------------------------------------------------------------------

    /// Reads the workspace list.
    pub fn workspaces<T>(&self, read: impl FnOnce(&WorkspaceStore) -> T) -> T {
        read(&self.workspaces.lock().expect("workspace mutex poisoned"))
    }

    /// Mutates the workspace list and persists it, keeping disk and memory in agreement.
    ///
    /// The change is only kept if the save succeeds — otherwise the in-memory list is
    /// rolled back, so what the user sees always matches what will be there next launch.
    pub fn update_workspaces<T>(
        &self,
        mutate: impl FnOnce(&mut WorkspaceStore) -> AppResult<T>,
    ) -> AppResult<T> {
        self.workspace_transaction(|store, dir| {
            let outcome = mutate(store)?;
            store.save(dir)?;
            Ok(outcome)
        })
    }

    /// The callback must persist the candidate registry and roll back its filesystem work
    /// on failure. This wrapper restores only the in-memory registry and serializes writers.
    pub fn workspace_transaction<T>(
        &self,
        mutate_and_save: impl FnOnce(&mut WorkspaceStore, &Path) -> AppResult<T>,
    ) -> AppResult<T> {
        let mut guard = self.workspaces.lock().expect("workspace mutex poisoned");
        guard.ensure_writable()?;
        let snapshot = guard.clone();
        let outcome = match mutate_and_save(&mut guard, &self.paths.app_data_dir) {
            Ok(outcome) => outcome,
            Err(err) => {
                *guard = snapshot;
                return Err(err);
            }
        };
        Ok(outcome)
    }

    /// The folder notebooks are created in and JupyterLab is rooted at right now.
    ///
    /// Preserve the selected path even if its drive is disconnected. Silently switching
    /// roots could open or create a same-named notebook in another course.
    pub fn active_workspace_dir(&self) -> PathBuf {
        self.workspaces(|store| store.active_record().map(|record| record.path.clone()))
            .unwrap_or_else(|| self.paths.workspace_dir.clone())
    }

    pub fn require_active_workspace(&self) -> AppResult<PathBuf> {
        let path = self.active_workspace_dir();
        if !path.is_dir() {
            return Err(AppError::new("workspace", "WORKSPACE_FOLDER_MISSING", "Your workspace folder is unavailable",
                "Reconnect its drive, or launch another workspace from Home. Your folders have not been moved or replaced."));
        }
        Ok(path)
    }

    // --- notebook servers ------------------------------------------------------------------

    /// An authenticated session rooted at `root`, starting a server if one isn't running.
    ///
    /// Servers are matched by folder, so returning to a workspace reuses its existing
    /// session — including any kernels still computing in it.
    pub fn session_for(&self, root: &Path) -> AppResult<JupyterSession> {
        let mut guard = self.jupyter.lock().expect("jupyter mutex poisoned");

        // Drop servers that have died (crash, environment stopped, distro terminated)
        // before deciding anything: a dead entry would otherwise hand back a URL pointing
        // at a closed port, which presents as an unexplained blank page.
        guard.retain_mut(|server| !server.has_exited());

        if let Some(server) = guard.iter().find(|s| workspaces::same_path(&s.root, root)) {
            if !crate::jupyter::status_ready(&server.session) {
                return Err(AppError::new("jupyter", "SERVER_UNRESPONSIVE", "Your notebook service isn't responding yet",
                    "Wait a moment and try again. If it stays unresponsive, save any open work and use Stop SageMath before reopening the workspace."));
            }
            return Ok(server.session.clone());
        }

        if guard.len() >= MAX_LIVE_SERVERS {
            return Err(AppError::new(
                "jupyter",
                "TOO_MANY_WORKSPACES_OPEN",
                "That's as many workspaces as SageDock can run at once",
                "Several workspaces are already open and running. Use Stop SageMath on the Home screen to close them all, then open the one you need. Your saved notebooks are safe.",
            ));
        }

        let mut server = crate::jupyter::start_ready(root)?;
        if let Err(err) = crate::jupyter::verify_kernels(&server.session) {
            server.stop();
            return Err(err);
        }
        let session = server.session.clone();
        guard.push(server);
        self.mark_runtime_owned();
        Ok(session)
    }

    /// Whether any notebook server is genuinely answering.
    /// Counts processes without treating a temporarily slow HTTP response as a crash.
    pub fn has_server(&self) -> bool {
        self.live_server_count() > 0
    }

    /// Whether a notebook server is rooted at this folder, or anywhere inside it.
    ///
    /// Scoped deliberately. Renaming a workspace moves its folder on disk, which would
    /// strand a server rooted there — but only *that* folder's server. Asking instead
    /// whether any server is running at all refused every rename the moment a single
    /// notebook was open anywhere, including one in an unrelated course. That is not a
    /// safety property, just a false one, and it is what made renaming appear broken.
    pub fn has_server_under(&self, path: &Path) -> bool {
        let target = path.canonicalize();
        let mut guard = self.jupyter.lock().expect("jupyter mutex poisoned");
        guard.iter_mut().any(|server| {
            if server.has_exited() {
                return false;
            }
            if workspaces::same_path(&server.root, path) {
                return true;
            }
            // A server rooted in a subfolder is stranded by a rename of the parent just as
            // surely as one rooted at it.
            match (&target, server.root.canonicalize()) {
                (Ok(target), Ok(root)) => root.starts_with(target),
                _ => server.root.starts_with(path),
            }
        })
    }

    pub fn live_server_count(&self) -> usize {
        self.jupyter
            .lock()
            .expect("jupyter mutex poisoned")
            .iter_mut()
            .map(|server| usize::from(!server.has_exited()))
            .sum()
    }

    /// Stops every notebook server, so a closed window doesn't leave one behind.
    pub fn shutdown_jupyter(&self) {
        let mut guard = self.jupyter.lock().expect("jupyter mutex poisoned");
        for server in guard.iter_mut() {
            server.stop();
        }
        guard.clear();
    }
}

/// The refusal shown when something else already holds the operation slot.
///
/// Names the operation actually in flight, because "SageDock is still creating a backup"
/// and "SageDock is still setting up SageMath" call for different patience.
pub fn busy_error(state: &AppState) -> AppError {
    let doing = state
        .current_operation()
        .unwrap_or("finishing another task");
    AppError::new(
        "app",
        "OPERATION_BUSY",
        "SageDock is busy for a moment",
        format!("SageDock is still {doing}. Wait for that to finish, then try again — your saved notebooks are safe."),
    )
}

/// Labels for `begin_operation`, each completing "SageDock is still …".
pub mod operation {
    pub const SETUP: &str = "setting up SageMath";
    pub const NOTEBOOK: &str = "opening a notebook";
    pub const TOOL: &str = "installing a scientific tool";
    pub const RECOVERY: &str = "repairing your computing environment";
    pub const BACKUP: &str = "creating a backup";
    pub const RESTORE: &str = "restoring a backup";
    pub const WORKSPACE: &str = "updating your workspaces";
    pub const STOPPING: &str = "stopping SageMath";
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

    fn test_state() -> AppState {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, AtomicOrdering::SeqCst);
        let dir = std::env::temp_dir().join(format!("sagedock-state-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        AppState::new(
            ConfigStore::new(&dir),
            AppPaths {
                app_data_dir: dir.clone(),
                workspace_dir: dir.join("SageDock"),
                package_search_dirs: Vec::new(),
            },
        )
    }

    /// An instance that never started the environment must leave it alone — this is what
    /// stops a second window's exit from killing the first window's notebooks.
    #[test]
    fn an_instance_that_did_not_start_the_runtime_may_not_stop_it() {
        let state = test_state();
        assert!(!state.may_stop_runtime());
    }

    #[test]
    fn the_instance_that_started_the_runtime_may_stop_it() {
        let state = test_state();
        state.mark_runtime_owned();
        assert!(state.may_stop_runtime());
    }

    /// Stopping the environment while setup is still writing to it would corrupt the import.
    #[test]
    fn an_operation_in_flight_blocks_stopping_the_runtime() {
        let state = test_state();
        state.mark_runtime_owned();

        let guard = state
            .begin_operation(operation::SETUP)
            .expect("slot should be free");
        assert!(!state.may_stop_runtime());

        drop(guard);
        assert!(
            state.may_stop_runtime(),
            "the slot should be released on drop"
        );
    }

    #[test]
    fn only_one_operation_runs_at_a_time() {
        let state = test_state();
        let first = state
            .begin_operation(operation::BACKUP)
            .expect("first claim should succeed");
        assert!(
            state.begin_operation(operation::SETUP).is_none(),
            "a second must be refused"
        );

        drop(first);
        assert!(
            state.begin_operation(operation::SETUP).is_some(),
            "reusable afterwards"
        );
    }

    /// The bug this guards against: every operation shared one flag named after setup, so
    /// creating a backup told the user SageDock was installing SageMath.
    #[test]
    fn a_backup_is_not_reported_as_setup() {
        let state = test_state();
        let _guard = state.begin_operation(operation::BACKUP).unwrap();

        assert!(state.is_busy());
        assert_ne!(
            state.current_operation(),
            Some(operation::SETUP),
            "a backup must not claim to be setup"
        );
        assert_eq!(state.current_operation(), Some(operation::BACKUP));
    }

    #[test]
    fn setup_is_still_distinguishable_from_other_work() {
        let state = test_state();
        let _guard = state.begin_operation(operation::SETUP).unwrap();
        assert_eq!(state.current_operation(), Some(operation::SETUP));
    }

    #[test]
    fn a_default_workspace_exists_on_a_fresh_installation() {
        let state = test_state();
        let views = state.workspaces(|store| store.views());

        assert_eq!(views.len(), 1);
        assert!(views[0].is_active);
    }

    /// A disconnected course must never redirect notebook access to another course.
    #[test]
    fn a_missing_active_workspace_never_switches_to_the_default() {
        let state = test_state();
        std::fs::create_dir_all(&state.paths.workspace_dir).unwrap();

        let ghost = state
            .update_workspaces(|store| {
                let record = crate::workspaces::WorkspaceRecord {
                    id: "ws-ghost".into(),
                    name: "Gone".into(),
                    path: state.paths.app_data_dir.join("does-not-exist"),
                    last_opened: None,
                };
                store.workspaces.push(record.clone());
                store.active = Some(record.id.clone());
                Ok(record)
            })
            .unwrap();

        assert_eq!(state.workspaces(|s| s.active.clone()), Some(ghost.id));
        assert_eq!(state.active_workspace_dir(), ghost.path);
        assert_eq!(
            state.require_active_workspace().unwrap_err().code,
            "WORKSPACE_FOLDER_MISSING"
        );
    }

    #[test]
    fn a_failed_workspace_change_leaves_the_list_untouched() {
        let state = test_state();
        let before = state.workspaces(|store| store.workspaces.clone());

        let result: AppResult<()> = state.update_workspaces(|store| {
            store.workspaces.clear();
            Err(AppError::new("workspace", "TEST", "no", "no"))
        });

        assert!(result.is_err());
        assert_eq!(state.workspaces(|store| store.workspaces.clone()), before);
    }

    #[test]
    fn workspace_changes_survive_a_restart() {
        let state = test_state();
        let dir = state.paths.app_data_dir.clone();
        let created = state
            .update_workspaces(|store| crate::workspaces::create(store, &dir, "Calculus"))
            .unwrap();

        let reloaded = WorkspaceStore::load(&dir);
        assert!(reloaded.workspaces.iter().any(|w| w.id == created.id));
    }
}
