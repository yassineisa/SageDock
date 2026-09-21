//! Folder-based workspaces: the user's own project folders, tracked by SageDock.
//!
//! A workspace is deliberately nothing more than an ordinary Windows directory. SageDock
//! records where it is and when it was last opened; it does not own the files, does not
//! move them somewhere private, and does not need them to be anywhere in particular. That
//! is what lets a student copy a workspace to a USB stick, keep it in OneDrive, or hand it
//! to a classmate, and it is why removing a workspace from the Home list never deletes
//! anything from disk.
//!
//! The list itself lives in one small JSON file next to the app's other state, written
//! atomically. A corrupt or missing file degrades to "no workspaces recorded" rather than
//! failing to launch — losing the *list* must never look like losing the *work*.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

const STORE_FILE: &str = "workspaces.json";

/// Bumped only when the on-disk shape changes incompatibly.
pub const STORE_FORMAT: u32 = 1;

/// Windows' own limit is 255, but a workspace name also becomes a path segment inside a
/// backup archive and inside the Linux view of the folder. A generous cap keeps all three
/// comfortably short.
const MAX_NAME_LEN: usize = 64;

/// Names Windows reserves for devices. Reserved with *any* extension, and case-insensitive,
/// so `con`, `CON`, and `Con.txt` are all refused.
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceRecord {
    /// Stable across renames and moves, so the active selection survives both.
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    /// Unix seconds. `None` until the workspace has been launched once.
    #[serde(default)]
    pub last_opened: Option<u64>,
}

/// What the Home screen renders. Availability is computed fresh on every read rather than
/// stored, because a folder can be renamed, unplugged, or unshared while the app is open.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceView {
    pub id: String,
    pub name: String,
    pub path: String,
    pub last_opened: Option<u64>,
    pub is_active: bool,
    /// `available`, `missing`, or `unreadable` — never a bare boolean, because "the folder
    /// is gone" and "the folder is there but I can't read it" need different advice.
    pub status: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceStore {
    #[serde(default = "default_format")]
    pub format: u32,
    #[serde(default)]
    pub active: Option<String>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceRecord>,
    #[serde(skip)]
    read_only: bool,
}

fn default_format() -> u32 {
    STORE_FORMAT
}

impl Default for WorkspaceStore {
    fn default() -> Self {
        Self {
            format: STORE_FORMAT,
            active: None,
            workspaces: Vec::new(),
            read_only: false,
        }
    }
}

impl WorkspaceStore {
    /// Reads the list. Never fails: an unreadable or corrupt file logs and yields an empty
    /// list, which `ensure_default` then repopulates with the standard workspace.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(STORE_FILE);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                tracing::warn!(target: "workspace", error = %err, "could not read the workspace list");
                return Self {
                    read_only: true,
                    ..Self::default()
                };
            }
        };

        match serde_json::from_str::<Self>(&raw) {
            // A file written by a newer SageDock may describe workspaces this build would
            // mishandle. Showing none is recoverable; rewriting it is not.
            Ok(store) if store.format > STORE_FORMAT => {
                tracing::warn!(target: "workspace", format = store.format, "workspace list is from a newer SageDock");
                Self {
                    read_only: true,
                    ..Self::default()
                }
            }
            Ok(store) => store,
            Err(err) => {
                tracing::warn!(target: "workspace", error = %err, "workspace list was unreadable");
                Self {
                    read_only: true,
                    ..Self::default()
                }
            }
        }
    }

    pub fn save(&self, dir: &Path) -> AppResult<()> {
        self.ensure_writable()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| store_error(e.to_string()))?;
        crate::storage::atomic_write(&dir.join(STORE_FILE), &bytes)
            .map_err(|e| store_error(e.to_string()))
    }

    pub fn ensure_writable(&self) -> AppResult<()> {
        if self.read_only {
            return Err(AppError::new("workspace", "WORKSPACE_LIST_PROTECTED", "Your saved workspace list needs attention",
                "SageDock couldn't safely read the list and has preserved it. Your notebook folders are safe. If you recently changed SageDock versions, reopen the newer version. Otherwise, contact support to recover the list."));
        }
        Ok(())
    }

    /// Guarantees the default workspace is present and something is selected.
    ///
    /// Run on every launch: it is what migrates an installation that predates workspaces,
    /// and what recovers from an emptied or corrupt list without the user noticing.
    pub fn ensure_default(&mut self, default_dir: &Path) -> bool {
        let mut changed = false;

        if self.workspaces.is_empty() {
            let name = default_dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "SageDock".into());
            self.workspaces.insert(
                0,
                WorkspaceRecord {
                    id: new_id(),
                    name,
                    path: default_dir.to_path_buf(),
                    last_opened: None,
                },
            );
            changed = true;
        }

        let active_is_valid = self
            .active
            .as_ref()
            .is_some_and(|id| self.workspaces.iter().any(|w| &w.id == id));
        if !active_is_valid {
            self.active = self.workspaces.first().map(|w| w.id.clone());
            changed = true;
        }

        changed
    }

    pub fn find(&self, id: &str) -> Option<&WorkspaceRecord> {
        self.workspaces.iter().find(|w| w.id == id)
    }

    pub fn active_record(&self) -> Option<&WorkspaceRecord> {
        self.active.as_ref().and_then(|id| self.find(id))
    }

    pub fn views(&self) -> Vec<WorkspaceView> {
        self.workspaces
            .iter()
            .map(|w| WorkspaceView {
                id: w.id.clone(),
                name: w.name.clone(),
                path: w.path.display().to_string(),
                last_opened: w.last_opened,
                is_active: self.active.as_deref() == Some(w.id.as_str()),
                status: status_of(&w.path),
            })
            .collect()
    }
}

/// Classifies a workspace folder without doing anything to it.
fn status_of(path: &Path) -> &'static str {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => {
            // Being able to enumerate is the cheapest honest proxy for "usable". A folder
            // on a disconnected network share exists but cannot be read.
            if std::fs::read_dir(path).is_ok() {
                "available"
            } else {
                "unreadable"
            }
        }
        Ok(_) => "unreadable",
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => "missing",
        Err(_) => "unreadable",
    }
}

fn new_id() -> String {
    let mut bytes = [0u8; 16];
    // A failure here would mean the OS CSPRNG is unavailable; a time-based id is a fine
    // fallback for what is only a local list key, never a security value.
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        return format!("ws-{nanos:032x}");
    }
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("ws-{hex}")
}

/// Compares two paths for "the same folder", tolerating case and separator differences.
///
/// Canonicalization is preferred because it resolves `..`, short 8.3 names, and mapped
/// drives, but it only works on a path that currently exists — so a missing folder falls
/// back to a case-insensitive comparison rather than being treated as a different one.
pub fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => {
            let norm = |p: &Path| {
                p.to_string_lossy()
                    .replace('/', "\\")
                    .trim_end_matches('\\')
                    .to_lowercase()
            };
            norm(a) == norm(b)
        }
    }
}

/// Validates a workspace name against Windows' filesystem rules.
///
/// Done here rather than relying on the OS to reject it, so the user gets a plain-language
/// reason before anything is created, instead of a raw `CreateDirectory` failure after.
pub fn validate_name(name: &str) -> AppResult<String> {
    let trimmed = name.trim();

    if trimmed.is_empty() {
        return Err(name_error(
            "Give your workspace a name, for example Calculus.",
        ));
    }
    if trimmed.chars().count() > MAX_NAME_LEN {
        return Err(name_error(format!(
            "That name is too long. Please use {MAX_NAME_LEN} characters or fewer."
        )));
    }
    if let Some(bad) = trimmed.chars().find(|c| r#"<>:"/\|?*"#.contains(*c)) {
        return Err(name_error(format!(
            "Windows folder names can't contain {bad}. Try using a space or a dash instead."
        )));
    }
    if trimmed.chars().any(|c| (c as u32) < 0x20) {
        return Err(name_error(
            "That name contains characters Windows doesn't allow in a folder name.",
        ));
    }
    // Windows silently strips these, so a folder named "Physics." becomes "Physics" and
    // the recorded path would stop matching what is actually on disk.
    if trimmed.ends_with('.') || trimmed.ends_with(' ') {
        return Err(name_error(
            "Windows folder names can't end with a dot or a space.",
        ));
    }
    let stem = trimmed
        .split('.')
        .next()
        .unwrap_or(trimmed)
        .to_ascii_uppercase();
    if RESERVED_NAMES.contains(&stem.as_str()) {
        return Err(name_error(format!(
            "Windows reserves the name {trimmed} for hardware. Please choose a different name."
        )));
    }

    Ok(trimmed.to_string())
}

/// Creates a new workspace folder beneath `parent` and records it.
pub fn create(store: &mut WorkspaceStore, parent: &Path, name: &str) -> AppResult<WorkspaceRecord> {
    let name = validate_name(name)?;
    let path = parent.join(&name);

    if store.workspaces.iter().any(|w| same_path(&w.path, &path)) {
        return Err(duplicate_error(&name));
    }
    // `create_dir_all` succeeds on an existing folder, which would silently adopt whatever
    // is already there. Asking first lets the user decide to add it instead.
    if path.exists() {
        return Err(AppError::new(
            "workspace",
            "WORKSPACE_FOLDER_EXISTS",
            "There's already a folder with that name",
            format!("A folder called {name} already exists in your SageDock folder. Choose a different name, or use Add existing folder to use the one that's already there."),
        ));
    }

    std::fs::create_dir(&path).map_err(|e| create_error(&path, e.to_string()))?;

    let record = WorkspaceRecord {
        id: new_id(),
        name,
        path,
        last_opened: None,
    };
    store.workspaces.push(record.clone());
    tracing::info!(target: "workspace", id = %record.id, "workspace created");
    Ok(record)
}

/// Records an existing folder the user chose in a native picker.
pub fn add_existing(store: &mut WorkspaceStore, path: &Path) -> AppResult<WorkspaceRecord> {
    if !path.is_dir() {
        return Err(AppError::new(
            "workspace",
            "WORKSPACE_NOT_A_FOLDER",
            "That isn't a folder SageDock can use",
            "Choose a folder rather than a file. SageDock keeps your notebooks in an ordinary Windows folder.",
        ));
    }
    // Rejected up front: the Linux side sees Windows drives through /mnt, and a network
    // or UNC location has no such mapping, so notebooks there would fail to open later
    // with a much more confusing error.
    crate::runtime::wsl::windows_path_to_wsl(path)?;

    if let Some(existing) = store.workspaces.iter().find(|w| same_path(&w.path, path)) {
        return Err(duplicate_error(&existing.name));
    }

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Workspace".into());
    let record = WorkspaceRecord {
        id: new_id(),
        name,
        path: path.to_path_buf(),
        last_opened: None,
    };
    store.workspaces.push(record.clone());
    tracing::info!(target: "workspace", id = %record.id, "existing folder added as a workspace");
    Ok(record)
}

/// Renames both the folder on disk and the entry, so the card and File Explorer agree.
///
/// Refuses rather than guessing when the destination is taken. The caller is responsible
/// for making sure no notebook server is serving this folder — renaming underneath a
/// running server would strand it on a path that no longer exists.
pub fn rename(store: &mut WorkspaceStore, id: &str, new_name: &str) -> AppResult<WorkspaceRecord> {
    let new_name = validate_name(new_name)?;
    let index = store
        .workspaces
        .iter()
        .position(|w| w.id == id)
        .ok_or_else(unknown_error)?;

    let current = store.workspaces[index].clone();
    if current.name == new_name {
        return Ok(current);
    }

    let target = match current.path.parent() {
        Some(parent) => parent.join(&new_name),
        None => return Err(unknown_error()),
    };

    if store
        .workspaces
        .iter()
        .enumerate()
        .any(|(i, w)| i != index && same_path(&w.path, &target))
    {
        return Err(duplicate_error(&new_name));
    }

    // A folder that has gone missing can still be renamed in the list; there is nothing on
    // disk to move, and refusing would leave the user stuck with a stale name.
    if current.path.is_dir() {
        let old_root = current
            .path
            .canonicalize()
            .map_err(|e| create_error(&current.path, e.to_string()))?;
        let children: Vec<(usize, PathBuf)> = store
            .workspaces
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .filter_map(|(i, record)| {
                let child = record
                    .path
                    .canonicalize()
                    .unwrap_or_else(|_| record.path.clone());
                child
                    .strip_prefix(&old_root)
                    .or_else(|_| record.path.strip_prefix(&current.path))
                    .ok()
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(|p| (i, target.join(p)))
            })
            .collect();
        if target.exists() && !same_path(&current.path, &target) {
            return Err(AppError::new(
                "workspace",
                "WORKSPACE_FOLDER_EXISTS",
                "There's already a folder with that name",
                format!("A folder called {new_name} already exists alongside this one. Choose a different name."),
            ));
        }
        std::fs::rename(&current.path, &target).map_err(|err| {
            AppError::new(
                "workspace",
                "WORKSPACE_RENAME_FAILED",
                "SageDock couldn't rename that workspace",
                "The folder couldn't be renamed. This usually means a file inside it is open in another program. Close any open notebooks and try again — nothing has been changed.",
            )
            .with_technical_details(format!("{} -> {}: {err}", current.path.display(), target.display()))
        })?;
        store.workspaces[index].path = target;
        for (i, path) in children {
            store.workspaces[i].path = path;
        }
    }

    store.workspaces[index].name = new_name;
    Ok(store.workspaces[index].clone())
}

/// Removes a workspace from the list. Never touches the folder or anything inside it.
pub fn forget(store: &mut WorkspaceStore, id: &str) -> AppResult<()> {
    let index = store
        .workspaces
        .iter()
        .position(|w| w.id == id)
        .ok_or_else(unknown_error)?;

    if store.workspaces.len() == 1 {
        return Err(AppError::new(
            "workspace",
            "WORKSPACE_LAST_REMAINING",
            "This is your only workspace",
            "SageDock needs at least one workspace. Create or add another one first, then you can remove this one from the list. Your files are not affected either way.",
        ));
    }

    let removed = store.workspaces.remove(index);
    if store.active.as_deref() == Some(removed.id.as_str()) {
        store.active = store.workspaces.first().map(|w| w.id.clone());
    }
    tracing::info!(target: "workspace", id = %removed.id, "workspace removed from the list (files kept)");
    Ok(())
}

pub fn set_active(store: &mut WorkspaceStore, id: &str) -> AppResult<WorkspaceRecord> {
    let record = store.find(id).ok_or_else(unknown_error)?.clone();
    store.active = Some(record.id.clone());
    Ok(record)
}

/// Stamps a workspace as opened now, for the "last opened" line on its card.
pub fn touch(store: &mut WorkspaceStore, id: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    if let Some(record) = store.workspaces.iter_mut().find(|w| w.id == id) {
        record.last_opened = Some(now);
    }
}

// --- errors ---------------------------------------------------------------------------

fn name_error(message: impl Into<String>) -> AppError {
    AppError::new(
        "workspace",
        "WORKSPACE_NAME_INVALID",
        "That name won't work as a folder",
        message,
    )
}

fn duplicate_error(name: &str) -> AppError {
    AppError::new(
        "workspace",
        "WORKSPACE_DUPLICATE",
        "That folder is already on your list",
        format!("SageDock is already using this folder as {name}. Look for it among your workspaces on the Home screen."),
    )
}

fn unknown_error() -> AppError {
    AppError::new(
        "workspace",
        "WORKSPACE_UNKNOWN",
        "SageDock couldn't find that workspace",
        "That workspace is no longer on your list. Return to the Home screen to see your current workspaces. Your files are not affected.",
    )
}

fn create_error(path: &Path, details: String) -> AppError {
    AppError::new(
        "workspace",
        "WORKSPACE_CREATE_FAILED",
        "SageDock couldn't create that workspace",
        "The folder couldn't be created. Check that you have free disk space and permission to save in your Documents folder.",
    )
    .with_technical_details(format!("{}: {details}", path.display()))
}

fn store_error(details: String) -> AppError {
    AppError::new(
        "workspace",
        "WORKSPACE_LIST_SAVE_FAILED",
        "SageDock couldn't save your workspace list",
        "Your folders and notebooks are safe and unchanged. SageDock couldn't record the change to its list of workspaces. Check available storage and try again.",
    )
    .with_technical_details(details)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("sagedock-ws-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn accepts_ordinary_course_names() {
        for name in ["Calculus", "Physics 101", "Stats-2026", "Linear Algebra"] {
            assert!(validate_name(name).is_ok(), "{name} should be allowed");
        }
    }

    /// Each of these would either be rejected by Windows or silently altered by it, and a
    /// silently altered name is worse: the recorded path stops matching the real folder.
    #[test]
    fn rejects_names_windows_cannot_store_faithfully() {
        for name in [
            "", "   ", "a/b", "a\\b", "what?", "a:b", "quote\"", "pipe|", "star*", "<lt>",
        ] {
            assert!(validate_name(name).is_err(), "{name:?} should be refused");
        }
        assert!(
            validate_name("Physics.").is_err(),
            "a trailing dot is stripped by Windows"
        );
        assert!(
            validate_name("Physics ").is_ok(),
            "a trailing space is trimmed, not refused"
        );
        assert!(validate_name(&"x".repeat(MAX_NAME_LEN + 1)).is_err());
    }

    #[test]
    fn rejects_reserved_device_names_in_any_casing_or_extension() {
        for name in ["CON", "con", "Nul", "com1", "LPT9", "con.txt"] {
            assert!(
                validate_name(name).is_err(),
                "{name} is reserved by Windows"
            );
        }
        assert!(
            validate_name("Console").is_ok(),
            "only the exact device names are reserved"
        );
    }

    #[test]
    fn names_are_trimmed_rather_than_stored_with_padding() {
        assert_eq!(validate_name("  Calculus  ").unwrap(), "Calculus");
    }

    #[test]
    fn creating_a_workspace_makes_one_empty_folder() {
        let parent = temp_dir();
        let mut store = WorkspaceStore::default();

        let record = create(&mut store, &parent, "Calculus").unwrap();

        assert!(record.path.is_dir());
        assert_eq!(std::fs::read_dir(&record.path).unwrap().count(), 0);
        assert_eq!(store.workspaces.len(), 1);
    }

    #[test]
    fn creating_over_an_existing_folder_is_refused_rather_than_adopting_it() {
        let parent = temp_dir();
        std::fs::create_dir_all(parent.join("Physics")).unwrap();
        std::fs::write(parent.join("Physics").join("coursework.ipynb"), b"work").unwrap();
        let mut store = WorkspaceStore::default();

        let err = create(&mut store, &parent, "Physics").unwrap_err();

        assert_eq!(err.code, "WORKSPACE_FOLDER_EXISTS");
        assert_eq!(
            std::fs::read(parent.join("Physics").join("coursework.ipynb")).unwrap(),
            b"work"
        );
    }

    #[test]
    fn the_same_folder_cannot_be_added_twice() {
        let parent = temp_dir();
        let mut store = WorkspaceStore::default();
        let record = create(&mut store, &parent, "Statistics").unwrap();

        assert_eq!(
            add_existing(&mut store, &record.path).unwrap_err().code,
            "WORKSPACE_DUPLICATE"
        );
        assert_eq!(store.workspaces.len(), 1);
    }

    /// The data-safety rule for this module: forgetting is a list operation, not a delete.
    #[test]
    fn forgetting_a_workspace_keeps_every_file() {
        let parent = temp_dir();
        let mut store = WorkspaceStore::default();
        let keep = create(&mut store, &parent, "Calculus").unwrap();
        let drop = create(&mut store, &parent, "Physics").unwrap();
        let notebook = drop.path.join("lab.ipynb");
        std::fs::write(&notebook, b"hours of work").unwrap();

        forget(&mut store, &drop.id).unwrap();

        assert!(drop.path.is_dir(), "the folder must survive");
        assert_eq!(std::fs::read(&notebook).unwrap(), b"hours of work");
        assert_eq!(store.workspaces.len(), 1);
        assert_eq!(store.workspaces[0].id, keep.id);
    }

    #[test]
    fn the_last_workspace_cannot_be_removed() {
        let parent = temp_dir();
        let mut store = WorkspaceStore::default();
        let only = create(&mut store, &parent, "Calculus").unwrap();

        assert_eq!(
            forget(&mut store, &only.id).unwrap_err().code,
            "WORKSPACE_LAST_REMAINING"
        );
    }

    #[test]
    fn removing_the_active_workspace_selects_another() {
        let parent = temp_dir();
        let mut store = WorkspaceStore::default();
        let first = create(&mut store, &parent, "Calculus").unwrap();
        let second = create(&mut store, &parent, "Physics").unwrap();
        set_active(&mut store, &second.id).unwrap();

        forget(&mut store, &second.id).unwrap();

        assert_eq!(store.active.as_deref(), Some(first.id.as_str()));
    }

    #[test]
    fn renaming_moves_the_folder_so_the_card_and_explorer_agree() {
        let parent = temp_dir();
        let mut store = WorkspaceStore::default();
        let record = create(&mut store, &parent, "Calc").unwrap();
        // Keep coverage for legacy or user-created nested folders.
        std::fs::create_dir(record.path.join("Notebooks")).unwrap();
        std::fs::write(record.path.join("Notebooks").join("week1.ipynb"), b"cells").unwrap();

        let renamed = rename(&mut store, &record.id, "Calculus").unwrap();

        assert_eq!(renamed.name, "Calculus");
        assert!(!record.path.exists(), "the old folder should be gone");
        assert!(renamed.path.is_dir());
        assert_eq!(
            std::fs::read(renamed.path.join("Notebooks").join("week1.ipynb")).unwrap(),
            b"cells",
            "renaming must carry the files with it"
        );
        assert_eq!(renamed.id, record.id, "the id must survive a rename");
    }

    #[test]
    fn renaming_onto_an_existing_folder_is_refused() {
        let parent = temp_dir();
        let mut store = WorkspaceStore::default();
        let first = create(&mut store, &parent, "Calculus").unwrap();
        create(&mut store, &parent, "Physics").unwrap();

        assert!(rename(&mut store, &first.id, "Physics").is_err());
        assert!(
            first.path.is_dir(),
            "the original must be untouched after a refused rename"
        );
    }

    #[test]
    fn a_missing_folder_reports_missing_and_an_existing_one_reports_available() {
        let parent = temp_dir();
        let mut store = WorkspaceStore::default();
        let record = create(&mut store, &parent, "Calculus").unwrap();
        store.active = Some(record.id.clone());

        assert_eq!(store.views()[0].status, "available");
        assert!(store.views()[0].is_active);

        std::fs::remove_dir_all(&record.path).unwrap();
        assert_eq!(store.views()[0].status, "missing");
    }

    #[test]
    fn the_default_workspace_is_created_for_an_installation_that_predates_workspaces() {
        let parent = temp_dir();
        let default_dir = parent.join("SageDock");
        let mut store = WorkspaceStore::default();

        assert!(store.ensure_default(&default_dir));

        assert_eq!(store.workspaces.len(), 1);
        assert_eq!(store.workspaces[0].name, "SageDock");
        assert_eq!(
            store.active.as_deref(),
            Some(store.workspaces[0].id.as_str())
        );
        // Running it again must not add a second copy.
        assert!(!store.ensure_default(&default_dir));
        assert_eq!(store.workspaces.len(), 1);
    }

    #[test]
    fn an_active_id_pointing_at_nothing_is_repaired() {
        let parent = temp_dir();
        let default_dir = parent.join("SageDock");
        let mut store = WorkspaceStore {
            active: Some("ws-gone".into()),
            ..WorkspaceStore::default()
        };

        store.ensure_default(&default_dir);

        assert_eq!(
            store.active.as_deref(),
            Some(store.workspaces[0].id.as_str())
        );
    }

    #[test]
    fn the_list_survives_a_save_and_load_round_trip() {
        let dir = temp_dir();
        let mut store = WorkspaceStore::default();
        let record = create(&mut store, &dir, "Calculus").unwrap();
        set_active(&mut store, &record.id).unwrap();
        touch(&mut store, &record.id);
        store.save(&dir).unwrap();

        let loaded = WorkspaceStore::load(&dir);

        assert_eq!(loaded.workspaces, store.workspaces);
        assert_eq!(loaded.active, store.active);
        assert!(loaded.workspaces[0].last_opened.is_some());
    }

    /// Losing the list must never look like losing the work.
    #[test]
    fn a_corrupt_list_degrades_to_empty_instead_of_failing() {
        let dir = temp_dir();
        std::fs::write(dir.join(STORE_FILE), b"{ not json").unwrap();

        let loaded = WorkspaceStore::load(dir.as_path());

        assert!(loaded.workspaces.is_empty());
        assert_eq!(loaded.format, STORE_FORMAT);
    }

    #[test]
    fn a_list_from_a_newer_sagedock_is_not_reinterpreted() {
        let dir = temp_dir();
        std::fs::write(
            dir.join(STORE_FILE),
            format!(
                r#"{{"format":{},"active":null,"workspaces":[]}}"#,
                STORE_FORMAT + 1
            ),
        )
        .unwrap();

        let original = std::fs::read(dir.join(STORE_FILE)).unwrap();
        let mut loaded = WorkspaceStore::load(dir.as_path());
        assert!(loaded.workspaces.is_empty());
        loaded.ensure_default(&dir.join("default"));
        assert!(loaded.save(&dir).is_err());
        assert_eq!(std::fs::read(dir.join(STORE_FILE)).unwrap(), original);
    }

    #[test]
    fn forgotten_default_does_not_return_after_relaunch() {
        let dir = temp_dir();
        let default = dir.join("default");
        let mut store = WorkspaceStore::default();
        store.ensure_default(&default);
        let first = store.workspaces[0].id.clone();
        create(&mut store, &dir, "Physics").unwrap();
        forget(&mut store, &first).unwrap();
        store.save(&dir).unwrap();
        let mut loaded = WorkspaceStore::load(&dir);
        assert!(!loaded.ensure_default(&default));
        assert_eq!(loaded.workspaces.len(), 1);
        assert_eq!(loaded.workspaces[0].name, "Physics");
    }

    #[test]
    fn renaming_a_parent_keeps_registered_child_workspaces_connected() {
        let dir = temp_dir();
        let mut store = WorkspaceStore::default();
        let parent = create(&mut store, &dir, "Courses").unwrap();
        let child = create(&mut store, &parent.path, "Physics").unwrap();
        std::fs::write(child.path.join("lab.txt"), "keep").unwrap();
        rename(&mut store, &parent.id, "Classes").unwrap();
        let new_child = &store.find(&child.id).unwrap().path;
        assert_eq!(*new_child, dir.join("Classes/Physics"));
        assert_eq!(
            std::fs::read_to_string(new_child.join("lab.txt")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn paths_compare_equal_regardless_of_case_or_trailing_separator() {
        assert!(same_path(
            Path::new(r"C:\Users\A\Docs"),
            Path::new(r"c:\users\a\docs\")
        ));
        assert!(!same_path(
            Path::new(r"C:\Users\A\Docs"),
            Path::new(r"C:\Users\A\Other")
        ));
    }
}
