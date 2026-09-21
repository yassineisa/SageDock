//! Files the built-in browser downloaded, wherever the student chose to save them.
//!
//! Home's Downloads section is a scan of the Windows Downloads folder. That is enough while
//! every download lands there, but SageDock now asks where to save a download, so "somewhere
//! else" is the normal case. This is the small registry that keeps those files findable.
//!
//! It is deliberately **not** an index of the disk. Nothing here scans anywhere, and an entry
//! is only ever added by a download this application performed. The list is a list of paths
//! SageDock already knew, not a discovery mechanism.
//!
//! Persisted next to the workspace list and written the same way: `load` never fails, a file
//! from a newer SageDock is treated as read-only rather than rewritten, and saves go through
//! `storage::atomic_write`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult, ErrorSeverity};

const STORE_FILE: &str = "downloads.json";

pub const STORE_FORMAT: u32 = 1;

/// How many downloads are remembered.
///
/// This exists so the file cannot grow without bound over years of use. Entries are dropped
/// oldest-first, and dropping one only means it stops being listed on Home — the downloaded
/// file itself is never touched.
const MAX_TRACKED: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadStore {
    #[serde(default = "default_format")]
    pub format: u32,
    /// Most recently downloaded first.
    #[serde(default)]
    files: Vec<PathBuf>,
    #[serde(skip)]
    read_only: bool,
}

fn default_format() -> u32 {
    STORE_FORMAT
}

impl Default for DownloadStore {
    fn default() -> Self {
        Self {
            format: STORE_FORMAT,
            files: Vec::new(),
            read_only: false,
        }
    }
}

impl DownloadStore {
    /// Reads the list. Never fails: an unreadable, corrupt, or newer-format file yields an
    /// empty read-only list, because failing to remember a download must not stop SageDock
    /// from starting.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(STORE_FILE);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                tracing::warn!(target: "notebook", error = %err, "could not read the download list");
                return Self {
                    read_only: true,
                    ..Self::default()
                };
            }
        };

        match serde_json::from_str::<Self>(&raw) {
            Ok(store) if store.format > STORE_FORMAT => {
                tracing::warn!(target: "notebook", format = store.format, "download list is from a newer SageDock");
                Self {
                    read_only: true,
                    ..Self::default()
                }
            }
            Ok(store) => store,
            Err(err) => {
                tracing::warn!(target: "notebook", error = %err, "download list was unreadable");
                Self {
                    read_only: true,
                    ..Self::default()
                }
            }
        }
    }

    pub fn save(&self, dir: &Path) -> AppResult<()> {
        if self.read_only {
            return Err(store_error(
                "the download list was not readable, so it is not being overwritten".into(),
            ));
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| store_error(e.to_string()))?;
        crate::storage::atomic_write(&dir.join(STORE_FILE), &bytes)
            .map_err(|e| store_error(e.to_string()))
    }

    /// Remembers one downloaded file, most recent first.
    ///
    /// Downloading to the same path twice moves that one entry back to the top rather than
    /// listing it twice, which is what a student re-downloading a corrected file expects.
    pub fn record(&mut self, path: PathBuf) {
        self.files.retain(|existing| existing != &path);
        self.files.insert(0, path);
        self.files.truncate(MAX_TRACKED);
    }

    /// Every remembered file that is still on disk.
    ///
    /// Files deleted or moved outside SageDock are skipped rather than reported, so the
    /// Downloads list never offers something that cannot be opened.
    pub fn existing(&self) -> Vec<PathBuf> {
        self.files
            .iter()
            .filter(|path| path.is_file())
            .cloned()
            .collect()
    }

    /// Stops tracking one file. Returns whether anything changed, so the caller can skip a
    /// pointless write. Never deletes the file itself.
    pub fn forget(&mut self, path: &Path) -> bool {
        let before = self.files.len();
        self.files.retain(|existing| existing != path);
        before != self.files.len()
    }
}

fn store_error(technical_details: String) -> AppError {
    AppError::new(
        "notebook",
        "DOWNLOAD_LIST_WRITE_FAILED",
        "SageDock couldn't update its list of downloads",
        "The file downloaded successfully, but SageDock couldn't add it to the Downloads list on the Home screen. You can still find it in the folder you saved it to.",
    )
    .with_severity(ErrorSeverity::Warning)
    .with_technical_details(technical_details)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "sagedock-downloads-test-{}-{}",
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_newest_download_is_listed_first() {
        let mut store = DownloadStore::default();
        store.record(PathBuf::from(r"C:\a\one.ipynb"));
        store.record(PathBuf::from(r"C:\a\two.ipynb"));

        assert_eq!(
            store.files,
            vec![
                PathBuf::from(r"C:\a\two.ipynb"),
                PathBuf::from(r"C:\a\one.ipynb")
            ]
        );
    }

    /// Re-downloading a corrected file over the same path must not list it twice.
    #[test]
    fn downloading_to_the_same_path_twice_keeps_one_entry() {
        let mut store = DownloadStore::default();
        store.record(PathBuf::from(r"C:\a\one.ipynb"));
        store.record(PathBuf::from(r"C:\a\two.ipynb"));
        store.record(PathBuf::from(r"C:\a\one.ipynb"));

        assert_eq!(store.files.len(), 2);
        assert_eq!(store.files[0], PathBuf::from(r"C:\a\one.ipynb"));
    }

    #[test]
    fn the_list_is_capped_and_drops_the_oldest() {
        let mut store = DownloadStore::default();
        for n in 0..MAX_TRACKED + 10 {
            store.record(PathBuf::from(format!(r"C:\a\{n}.ipynb")));
        }

        assert_eq!(store.files.len(), MAX_TRACKED);
        assert_eq!(
            store.files[0],
            PathBuf::from(format!(r"C:\a\{}.ipynb", MAX_TRACKED + 9))
        );
        assert!(!store.files.contains(&PathBuf::from(r"C:\a\0.ipynb")));
    }

    /// A download the student has since deleted or moved must not be offered on Home.
    #[test]
    fn files_that_are_no_longer_there_are_not_reported() {
        let dir = temp_dir();
        let present = dir.join("here.ipynb");
        std::fs::write(&present, b"{}").unwrap();

        let mut store = DownloadStore::default();
        store.record(dir.join("gone.ipynb"));
        store.record(present.clone());

        assert_eq!(store.existing(), vec![present]);
    }

    #[test]
    fn forgetting_reports_whether_anything_changed_and_keeps_the_file() {
        let dir = temp_dir();
        let file = dir.join("keep.ipynb");
        std::fs::write(&file, b"{}").unwrap();

        let mut store = DownloadStore::default();
        store.record(file.clone());

        assert!(store.forget(&file));
        assert!(!store.forget(&file));
        assert!(file.is_file(), "forgetting must never delete the file");
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = temp_dir();
        let mut store = DownloadStore::default();
        store.record(PathBuf::from(r"C:\a\one.ipynb"));
        store.save(&dir).expect("save should succeed");

        let loaded = DownloadStore::load(&dir);

        assert_eq!(loaded.files, vec![PathBuf::from(r"C:\a\one.ipynb")]);
    }

    #[test]
    fn load_returns_an_empty_list_when_the_file_is_missing() {
        assert!(DownloadStore::load(&temp_dir()).files.is_empty());
    }

    /// A corrupt list must not be silently rewritten over: it yields nothing and refuses to
    /// save, the same rule the workspace list follows.
    #[test]
    fn a_corrupt_list_is_empty_and_refuses_to_overwrite_itself() {
        let dir = temp_dir();
        std::fs::write(dir.join(STORE_FILE), b"{ not json").unwrap();

        let store = DownloadStore::load(&dir);

        assert!(store.files.is_empty());
        assert!(store.save(&dir).is_err());
    }

    /// A list written by a newer SageDock may mean something this build would mishandle.
    #[test]
    fn a_newer_format_is_not_overwritten() {
        let dir = temp_dir();
        std::fs::write(
            dir.join(STORE_FILE),
            br#"{"format":99,"files":["C:\\a\\one.ipynb"]}"#,
        )
        .unwrap();

        let store = DownloadStore::load(&dir);

        assert!(store.files.is_empty());
        assert!(store.save(&dir).is_err());
    }
}
