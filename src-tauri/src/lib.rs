// IPC errors carry user guidance and diagnostics by value. These cold error paths favor
// a stable serializable contract over boxing every command's error solely for stack size.
#![allow(clippy::result_large_err)]

mod backup;
mod browsers;
mod commands;
mod config;
mod desktop;
mod downloads;
#[cfg(test)]
mod e2e;
mod error;
mod home;
mod jupyter;
mod library;
mod logging;
mod runtime;
mod scientific;
mod state;
mod storage;
mod system;
mod workspaces;

use tauri::{Emitter, Manager, RunEvent};

use config::ConfigStore;
use state::{AppPaths, AppState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Registered first, as the plugin requires. One SageDock owns the environment at a
        // time: a second launch hands focus to the existing window and exits, instead of
        // becoming a second owner whose exit would stop the first one's notebooks.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let log_dir = app
                .path()
                .app_log_dir()
                .expect("app log directory should be resolvable");
            // Leaked intentionally: the guard must outlive the app, and `run()` never
            // returns until the process exits, so there is no later point to drop it at.
            let guard = logging::init_logging(&log_dir);
            Box::leak(Box::new(guard));

            let path = app.path();
            let config_dir = path
                .app_config_dir()
                .expect("app config directory should be resolvable");
            let app_data_dir = path
                .app_data_dir()
                .expect("app data directory should be resolvable");

            // Documents needs a resolution chain, not a single lookup: on a machine with
            // OneDrive-redirected Documents the known-folder API returns an empty path,
            // which would drop the user's notebooks into AppData where they'd never find
            // them. See `resolve_documents_dir`.
            let documents_dir = runtime::workspace::resolve_documents_dir(path.document_dir().ok());
            if documents_dir.is_none() {
                tracing::warn!(
                    target: "launcher",
                    "could not resolve the Documents folder; falling back to app data"
                );
            }
            let workspace_dir = runtime::workspace::default_workspace_dir(
                &documents_dir.unwrap_or_else(|| app_data_dir.clone()),
            );

            // Most preferred first: a package bundled with the app, one sitting next to the
            // executable, then the two places a user most plausibly put one they copied.
            let mut package_search_dirs = Vec::new();
            if let Ok(dir) = path.resource_dir() {
                package_search_dirs.push(dir.join("runtime"));
            }
            if let Some(dir) = std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(Into::into))
            {
                package_search_dirs.push(dir);
            }
            package_search_dirs.extend(path.download_dir().ok());
            package_search_dirs.extend(path.desktop_dir().ok());

            tracing::info!(
                target: "launcher",
                version = env!("CARGO_PKG_VERSION"),
                workspace = %workspace_dir.display(),
                "SageDock starting"
            );

            app.manage(AppState::new(
                ConfigStore::new(&config_dir),
                AppPaths {
                    app_data_dir,
                    workspace_dir,
                    package_search_dirs,
                },
            ));

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    if let Some(state) = window.try_state::<AppState>() {
                        // Closing mid-operation, or while notebooks are open, asks first
                        // rather than silently ending somebody's calculation.
                        if state.is_busy() || state.has_server() {
                            api.prevent_close();
                            let _ = window.emit("close-requested", state.is_busy());
                        }
                    }
                }

                // Files dropped on the window are copied into the workspace the student is
                // working in. Handled here rather than in the webview so the absolute paths
                // Windows reports for a drop never reach the frontend — the same rule every
                // other file operation in SageDock follows.
                if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) =
                    event
                {
                    let paths = paths.clone();
                    let window = window.clone();
                    // Copying can take a while for a large dataset, and this handler runs on
                    // the UI thread, where blocking would freeze the window mid-drop.
                    std::thread::spawn(move || {
                        let Some(state) = window.try_state::<AppState>() else {
                            return;
                        };
                        let outcome = home::drop_files(&state, &paths);
                        let _ = window.emit("files-dropped", outcome);
                    });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::open_project_page,
            commands::get_config,
            commands::set_theme,
            commands::installed_browsers,
            commands::set_preferred_browser,
            commands::set_onboarding_complete,
            commands::get_setup_status,
            commands::choose_sage_package,
            commands::run_setup,
            commands::new_notebook,
            commands::shutdown_and_quit,
            commands::open_workspaces_folder,
            desktop::set_browser_preference,
            desktop::recent_notebooks,
            desktop::downloaded_notebooks,
            desktop::start_notebook_drag,
            desktop::choose_notebook_file,
            desktop::delete_recent_notebook,
            desktop::reveal_recent_notebook,
            desktop::delete_downloaded_file,
            desktop::open_downloaded_file_externally,
            desktop::reveal_downloaded_file,
            desktop::open_notebook,
            desktop::open_jupyter_home,
            desktop::scientific_tools,
            desktop::install_scientific_tool,
            desktop::recover,
            desktop::diagnostic_report,
            desktop::save_diagnostic_report,
            desktop::restart_windows,
            home::environment_status,
            home::stop_environment,
            home::list_workspaces,
            home::create_workspace,
            home::add_workspace_folder,
            home::rename_workspace,
            home::add_files_to_workspace,
            home::check_download_target,
            home::open_downloaded_notebook,
            home::check_picked_target,
            home::open_picked_notebook,
            home::forget_workspace,
            home::launch_workspace,
            home::reveal_workspace,
            home::create_backup,
            home::preview_backup,
            home::restore_backup,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // Closing the window must not leave a notebook server or a multi-gigabyte Linux
            // environment running in the background eating memory — but it must also not
            // stop an environment this instance does not own or is still working inside.
            if let RunEvent::ExitRequested { api, .. } = &event {
                if app.try_state::<AppState>().is_some_and(|s| s.is_busy()) {
                    api.prevent_exit();
                    return;
                }
            }
            if let RunEvent::Exit = event {
                if let Some(state) = app.try_state::<AppState>() {
                    state.shutdown_jupyter();
                    if state.may_stop_runtime() {
                        runtime::wsl::terminate_distro();
                    }
                }
            }
        });
}
