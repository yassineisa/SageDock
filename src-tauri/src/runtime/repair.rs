//! Runtime replacement is a recoverable transaction. Never deletes Windows workspaces.
use super::{
    health, image,
    provision::{self, PersistedSetupState, SetupPaths},
    workspace, wsl,
};
use crate::error::{AppError, AppResult};
use std::path::Path;

pub fn replace(paths: &SetupPaths, package: Option<&Path>, restore: bool) -> AppResult<()> {
    workspace::ensure_workspace(&paths.workspace_dir)?;
    workspace::verify_writable(&paths.workspace_dir)?;
    let recovery = paths.app_data_dir.join("recovery");
    std::fs::create_dir_all(&recovery).map_err(|e| failed(e.to_string()))?;
    let record = recovery.join("latest-backup.json");
    let backup = if restore {
        let bytes =
            std::fs::read(&record).map_err(|_| failed("No runtime backup is available yet"))?;
        let name: String = serde_json::from_slice(&bytes).map_err(|e| failed(e.to_string()))?;
        if !name.starts_with("runtime-")
            || !name.ends_with(".tar")
            || name.contains(['/', '\\', ':'])
        {
            return Err(failed("Invalid backup record"));
        }
        let path = recovery.join(name);
        if !path.is_file() {
            return Err(failed("The backup archive is missing"));
        }
        path
    } else {
        let package = package.ok_or_else(|| {
            failed("The approved SageMath package is required. Choose it on Home, then retry.")
        })?;
        image::verify(&image::inspect(package)?, &mut |_| {})?;
        wsl::ensure_owned(&paths.app_data_dir)?;
        health::require_space(&paths.app_data_dir, 30)?;
        wsl::stop_checked()?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let name = format!("runtime-{stamp}.tar");
        let backup = recovery.join(&name);
        wsl::export_backup(&backup)?;
        if std::fs::metadata(&backup).map(|m| m.len()).unwrap_or(0) < 1024 {
            return Err(failed("Backup was empty"));
        }
        let settings = serde_json::to_vec(&PersistedSetupState::load(&paths.app_data_dir))
            .map_err(|e| failed(e.to_string()))?;
        crate::storage::atomic_write(&recovery.join(format!("setup-{stamp}.json")), &settings)
            .map_err(|e| failed(e.to_string()))?;
        crate::storage::atomic_write(&record, &serde_json::to_vec(&name).unwrap())
            .map_err(|e| failed(e.to_string()))?;
        backup
    };
    health::require_space(&paths.app_data_dir, 15)?;
    let mut state = PersistedSetupState::load(&paths.app_data_dir);
    state.completed = false;
    state.save(&paths.app_data_dir)?;
    if wsl::distro_exists(wsl::distro_name()) {
        wsl::ensure_owned(&paths.app_data_dir)?;
        wsl::stop_checked()?;
        wsl::unregister_distro()?;
    }
    // The compilers belonged to the environment being replaced, so any remembered status is
    // now about software that no longer exists. Dropping it makes Home report "unknown"
    // until the new environment is probed, rather than claiming a tool is still installed.
    crate::scientific::forget_cache(&paths.app_data_dir);
    let archive = if restore {
        backup.as_path()
    } else {
        package.unwrap()
    };
    let result = wsl::import_distro(&wsl::default_install_dir(&paths.app_data_dir), archive)
        .and_then(|_| provision::run_setup(paths, None, &mut |_| {}).map(|_| ()));
    if let Err(original) = result {
        if !restore {
            // Restore the complete prior environment, including Linux-side customisations.
            if wsl::distro_exists(wsl::distro_name()) {
                wsl::ensure_owned(&paths.app_data_dir)?;
                wsl::stop_checked()?;
                wsl::unregister_distro()?;
            }
            wsl::import_distro(&wsl::default_install_dir(&paths.app_data_dir), &backup)
                .map_err(|_| failed("Automatic restore could not finish. The backup is retained; use Restore previous environment."))?;
        }
        return Err(failed(format!(
            "Replacement failed: {}. The runtime backup is retained. {}",
            original.code,
            if restore {
                "Try Restore previous environment again."
            } else {
                "The previous environment was reimported; run Check and repair to validate it."
            }
        )));
    }
    Ok(())
}

fn failed(details: impl Into<String>) -> AppError {
    AppError::new("repair", "RUNTIME_REPAIR_FAILED", "SageDock couldn't finish this repair",
        "Your Windows notebooks have not been removed. The previous environment backup, if created, is kept. View details for the next step, or export a diagnostic report.")
        .with_technical_details(details)
}
