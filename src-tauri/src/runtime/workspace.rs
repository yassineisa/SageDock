//! The Windows-side folder holding the user's notebooks and projects.
//!
//! This lives on the Windows side, outside the WSL environment, on purpose: it is the
//! single most important data-safety decision in the product. The Linux environment is
//! treated as disposable infrastructure that repair/reset/reinstall may destroy at any
//! time, while everything here survives all of that, stays visible in File Explorer, and
//! gets picked up by whatever backup tool (OneDrive and friends) the user already has.
//!
//! Nothing in this module deletes anything.

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

/// Resolves the default workspace location: `<Documents>\SageDock`.
pub fn default_workspace_dir(documents_dir: &Path) -> PathBuf {
    documents_dir.join("SageDock")
}

/// Finds the user's real Documents folder, trying several sources in order.
///
/// A single lookup is not good enough here, and this is not hypothetical: on a machine
/// whose Documents is redirected into OneDrive, the known-folder API this app's framework
/// uses returned an **empty** path, which silently sent the workspace into AppData — where
/// no user would ever find their notebooks, and where an app-data reset could take them
/// with it. The registry held the correct OneDrive path the whole time.
///
/// Order of preference:
/// 1. The known-folder path, when it's non-empty and actually exists.
/// 2. `User Shell Folders\Personal` — authoritative for redirected folders (OneDrive,
///    another drive, a roaming profile). Stored unexpanded, so `%USERPROFILE%`-style
///    placeholders are expanded here.
/// 3. `%USERPROFILE%\Documents`, the conventional layout.
///
/// Returns `None` only if all of them fail, letting the caller decide on a last resort
/// rather than silently picking somewhere surprising.
pub fn resolve_documents_dir(known_folder: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = known_folder {
        if !path.as_os_str().is_empty() && path.is_dir() {
            return Some(path);
        }
    }

    if let Some(path) = documents_from_registry() {
        if path.is_dir() {
            return Some(path);
        }
    }

    let profile = std::env::var("USERPROFILE").ok()?;
    let fallback = PathBuf::from(profile).join("Documents");
    fallback.is_dir().then_some(fallback)
}

fn documents_from_registry() -> Option<PathBuf> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders")
        .ok()?;

    let raw: String = key.get_value("Personal").ok()?;
    let expanded = expand_env_placeholders(&raw);

    (!expanded.is_empty()).then(|| PathBuf::from(expanded))
}

/// Expands `%NAME%` placeholders using the process environment.
///
/// `User Shell Folders` stores values as REG_EXPAND_SZ, so a value like
/// `%USERPROFILE%\Documents` arrives unexpanded and is meaningless as a literal path.
/// Unknown names are left untouched rather than replaced with an empty string, so a
/// mistake degrades to an obviously-wrong path instead of a silently-truncated one.
fn expand_env_placeholders(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];

        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) => out.push_str(&value),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                // Unpaired '%' — emit the remainder verbatim.
                out.push('%');
                out.push_str(after);
                return out;
            }
        }
    }

    out.push_str(rest);
    out
}

/// Creates only the workspace root. Existing files and user-chosen subfolders are preserved;
/// setup, health checks, and repair never impose a directory template.
pub fn ensure_workspace(workspace: &Path) -> AppResult<()> {
    std::fs::create_dir_all(workspace)
        .map_err(|err| workspace_error(workspace, err.to_string()))?;

    Ok(())
}

/// Confirms SageDock can actually write to the workspace, by writing and removing a probe
/// file. Checking permissions by inspecting ACLs is unreliable; attempting the write is
/// the only honest test. The probe is removed afterwards, and it never overwrites user
/// data because its name is SageDock-owned and unique per attempt.
pub fn verify_writable(workspace: &Path) -> AppResult<()> {
    use std::io::Write;
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|e| workspace_error(workspace, e.to_string()))?;
    let name: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    let probe = workspace.join(format!(".sagedock-write-test-{name}"));

    std::fs::OpenOptions::new().write(true).create_new(true).open(&probe)
        .and_then(|mut f| { f.write_all(b"ok")?; f.sync_all() }).map_err(|err| {
        AppError::new(
            "workspace",
            "WORKSPACE_NOT_WRITABLE",
            "SageDock can't save to your notebooks folder",
            "SageDock wasn't able to save a file to your SageDock folder. This can happen if the folder is read-only, or if a sync tool or antivirus is blocking it.",
        )
        .with_technical_details(format!("{}: {err}", probe.display()))
    })?;

    let read = std::fs::read(&probe).map_err(|e| workspace_error(workspace, e.to_string()))?;
    if read != b"ok" {
        return Err(workspace_error(
            workspace,
            "Read-back verification failed".into(),
        ));
    }
    std::fs::remove_file(&probe).map_err(|e| workspace_error(workspace, e.to_string()))?;
    Ok(())
}

fn workspace_error(path: &Path, details: String) -> AppError {
    AppError::new(
        "workspace",
        "WORKSPACE_CREATE_FAILED",
        "SageDock couldn't create your notebooks folder",
        "SageDock wasn't able to create the folder where your notebooks are kept. Check that you have free disk space and permission to save in your Documents folder.",
    )
    .with_technical_details(format!("{}: {details}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "sagedock-workspace-test-{}-{}",
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn workspace_path_sits_under_documents() {
        let docs = Path::new(r"C:\Users\someone\OneDrive\Documents");
        assert_eq!(
            default_workspace_dir(docs),
            PathBuf::from(r"C:\Users\someone\OneDrive\Documents\SageDock")
        );
    }

    #[test]
    fn creates_an_empty_workspace_root() {
        let workspace = temp_dir().join("SageDock");
        ensure_workspace(&workspace).unwrap();

        assert!(workspace.is_dir());
        assert_eq!(std::fs::read_dir(&workspace).unwrap().count(), 0);
    }

    /// Re-running setup must never disturb work that is already there.
    #[test]
    fn is_idempotent_and_preserves_existing_files() {
        let workspace = temp_dir().join("SageDock");
        ensure_workspace(&workspace).unwrap();

        // An existing layout from an older release must survive unchanged.
        std::fs::create_dir(workspace.join("Notebooks")).unwrap();
        let notebook = workspace.join("Notebooks").join("homework.ipynb");
        std::fs::write(&notebook, b"user work").unwrap();

        ensure_workspace(&workspace).unwrap();

        assert_eq!(std::fs::read(&notebook).unwrap(), b"user work");
    }

    #[test]
    fn expands_a_known_environment_placeholder() {
        std::env::set_var("SAGEDOCK_TEST_HOME", r"C:\Users\someone");
        assert_eq!(
            expand_env_placeholders(r"%SAGEDOCK_TEST_HOME%\Documents"),
            r"C:\Users\someone\Documents"
        );
    }

    #[test]
    fn leaves_unknown_placeholders_visible_rather_than_blanking_them() {
        let out = expand_env_placeholders(r"%SAGEDOCK_NOT_SET_ANYWHERE%\Documents");
        assert!(out.contains("SAGEDOCK_NOT_SET_ANYWHERE"), "got {out}");
    }

    #[test]
    fn leaves_plain_paths_untouched() {
        assert_eq!(
            expand_env_placeholders(r"C:\Users\someone\OneDrive\Documents"),
            r"C:\Users\someone\OneDrive\Documents"
        );
    }

    #[test]
    fn handles_an_unpaired_percent_without_panicking() {
        assert_eq!(expand_env_placeholders("100% done"), "100% done");
    }

    /// The exact failure seen on a real machine: the known-folder lookup came back empty,
    /// and the workspace must not be allowed to silently land in AppData.
    #[test]
    fn empty_known_folder_falls_through_to_another_source() {
        let resolved = resolve_documents_dir(Some(PathBuf::from("")));
        if let Some(path) = resolved {
            assert!(!path.as_os_str().is_empty());
            assert!(path.is_dir());
        }
    }

    #[test]
    fn a_valid_known_folder_is_preferred() {
        let dir = temp_dir();
        assert_eq!(resolve_documents_dir(Some(dir.clone())), Some(dir));
    }

    #[test]
    fn write_probe_cleans_up_after_itself() {
        let workspace = temp_dir();
        verify_writable(&workspace).unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(&workspace)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".sagedock-write-test")
            })
            .collect();

        assert!(leftovers.is_empty(), "probe file was left behind");
    }
}
