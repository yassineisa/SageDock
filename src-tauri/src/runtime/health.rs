//! Fast launch checks are distinct from setup's executing kernel tests.
use super::{workspace, wsl, PersistedSetupState};
use crate::{
    error::{AppError, AppResult, ErrorSeverity},
    system,
};
use std::path::Path;

pub fn preflight(data: &Path, work: &Path, importing: bool) -> AppResult<()> {
    // Preflight aborts only on Error/Fatal, and a restart advisory is never either, so this
    // does not gate setup and did not before. The persisted flag is passed so the restart
    // check can still distinguish a genuine setup-blocking restart from ordinary Windows
    // update noise rather than guessing. See `system::reboot`.
    let awaiting_restart = PersistedSetupState::load(data).awaiting_restart_now(data);
    let checks = system::run_system_check(awaiting_restart);
    for check in checks.checks.iter().filter(|c| c.id != "disk_space") {
        if matches!(check.severity, ErrorSeverity::Error | ErrorSeverity::Fatal) {
            return Err(AppError::new(
                "preflight",
                "COMPUTER_NOT_READY",
                &check.label,
                &check.summary,
            )
            .with_technical_details(check.detail.clone().unwrap_or_default()));
        }
    }
    require_space(data, if importing { 15 } else { 1 })?;
    require_space(work, 1)?;
    workspace::ensure_workspace(work)?;
    workspace::verify_writable(work)?;
    wsl::windows_path_to_wsl(work)?;
    std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| failure(e.to_string()))?;
    Ok(())
}

pub fn require_space(path: &Path, gib: u64) -> AppResult<()> {
    let bytes = system::disk_space::available(path)
        .ok_or_else(|| failure("Could not measure destination storage".into()))?;
    if bytes < gib * 1024 * 1024 * 1024 {
        return Err(AppError::new("storage", "LOW_DISK_SPACE", "A little more storage is needed",
            format!("SageDock needs {gib} GB free on this drive. About {:.1} GB is available. Free some space, then try again. Your saved notebooks are safe.", bytes as f64 / 1073741824.0)));
    }
    Ok(())
}

pub fn fast(workspace: &Path) -> AppResult<()> {
    if !wsl::distro_exists(wsl::distro_name()) {
        return Err(AppError::new("runtime", "ENVIRONMENT_MISSING", "Let's set up SageMath",
            "SageDock will install its own computing environment. Your existing files will stay where they are."));
    }
    workspace::ensure_workspace(workspace)?;
    workspace::verify_writable(workspace)?;
    // Fixed code; every value supplied by the host is a separate argument, never shell text.
    let code = r#"import json, pathlib, sys
from jupyter_client.kernelspec import KernelSpecManager
import jupyter_server, sage.version
m=json.loads(pathlib.Path('/opt/sagedock/runtime.json').read_text())
assert m['format'] == 1, 'Runtime needs a compatible SageDock version'
k=KernelSpecManager().find_kernel_specs()
assert all(n in k for n in ('sagemath','python3')), 'Notebook engines need repair'
p=pathlib.Path(sys.argv[1]); assert p.is_dir(), 'Workspace is not visible'
print('SAGEDOCK_HEALTH_OK')"#;
    let root = wsl::windows_path_to_wsl(workspace)?;
    let result = wsl::run_as(
        wsl::LINUX_USER,
        &[
            "/opt/sagedock/bin/sagedock-env",
            "python",
            "-c",
            code,
            &root,
        ],
    )?;
    if !result.success
        || !result
            .combined_output
            .lines()
            .any(|line| line == "SAGEDOCK_HEALTH_OK")
    {
        return Err(failure(result.combined_output));
    }
    Ok(())
}

fn failure(details: String) -> AppError {
    AppError::new("runtime", "HEALTH_CHECK_FAILED", "Your computing environment needs attention",
        "Your saved notebooks are safe. Open Recovery to check and repair SageMath, or try again if the computer was waking up.")
        .with_technical_details(details)
}
