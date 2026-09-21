//! Workspace-relative notebook access. Native pickers authorize imports; IPC paths
//! are resolved beneath the workspace and cannot escape via traversal or symlinks.
use crate::error::{AppError, AppResult};
use serde::Serialize;
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

#[derive(Serialize)]
pub struct NotebookEntry {
    /// How the frontend refers to this file. A workspace-relative path, a bare name in the
    /// Downloads folder, or an opaque `tracked:` key — never an absolute path.
    pub path: String,
    pub name: String,
    pub modified: u64,
    /// For a download saved outside the Downloads folder: the containing folder's **name**
    /// only, never its path. `None` for everything else.
    pub folder: Option<String>,
    /// Whether this is a Jupyter notebook, and so can be added to a workspace. Downloads
    /// from the built-in browser can be any file type; notebooks in a workspace always are.
    pub notebook: bool,
}

fn failure(details: impl Into<String>) -> AppError {
    AppError::new("notebook", "NOTEBOOK_ACCESS_FAILED", "This notebook couldn't be opened",
        "Your original file is safe. Choose a notebook in your SageDock folder, or use Open notebook to import a copy.")
        .with_technical_details(details)
}

pub fn resolve(root: &Path, relative: &str) -> AppResult<PathBuf> {
    let input = Path::new(relative);
    if relative.contains(':')
        || relative.contains('\0')
        || input
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(failure("Invalid relative notebook path"));
    }
    let base = root.canonicalize().map_err(|e| failure(e.to_string()))?;
    let file = base
        .join(input)
        .canonicalize()
        .map_err(|e| failure(e.to_string()))?;
    if !file.starts_with(&base)
        || !file.is_file()
        || file.extension().and_then(|s| s.to_str()) != Some("ipynb")
    {
        return Err(failure("Notebook must be inside the workspace"));
    }
    Ok(file)
}

pub fn recent(root: &Path) -> AppResult<Vec<NotebookEntry>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    fn scan(
        base: &Path,
        dir: &Path,
        depth: usize,
        out: &mut Vec<NotebookEntry>,
        budget: &mut usize,
    ) -> std::io::Result<()> {
        if depth > 6 {
            return Ok(());
        }
        for entry in fs::read_dir(dir)? {
            if *budget == 0 {
                break;
            }
            *budget -= 1;
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            // Includes Windows junctions, which must not redirect a scan outside the workspace.
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if metadata.is_dir() {
                scan(base, &entry.path(), depth + 1, out, budget)?;
            } else if entry.path().extension().and_then(|s| s.to_str()) == Some("ipynb") {
                out.push(NotebookEntry {
                    path: entry
                        .path()
                        .strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    name: name.trim_end_matches(".ipynb").to_owned(),
                    folder: None,
                    notebook: true,
                    modified: metadata
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                });
            }
        }
        Ok(())
    }
    let mut entries = Vec::new();
    scan(root, root, 0, &mut entries, &mut 5000).map_err(|e| failure(e.to_string()))?;
    entries.sort_by_key(|e| std::cmp::Reverse(e.modified));
    entries.truncate(30);
    Ok(entries)
}

/// The workspace file name a downloaded notebook would land as, before any dedup numbering.
///
/// Shared by the Downloads listing (for the name shown to the student), the copy-in path,
/// and the conflict check, so all three agree on what "the same notebook" means. A browser
/// commonly saves a notebook as `Report.json` or `Report.ipynb.json`, because Jupyter serves
/// `.ipynb` as `application/json`; both collapse to `Report.ipynb` here.
pub fn target_notebook_name(file_name: &str) -> String {
    let without_json = file_name.strip_suffix(".json").unwrap_or(file_name);
    let stem = without_json.strip_suffix(".ipynb").unwrap_or(without_json);
    format!("{stem}.ipynb")
}

/// Reads a notebook file and checks it really is one, without deciding where it goes.
fn read_and_validate_notebook(source: &Path) -> AppResult<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(source)
        .and_then(|f| f.take(100 * 1024 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|e| failure(e.to_string()))?;
    if bytes.len() > 100 * 1024 * 1024 {
        return Err(failure(
            "Notebook exceeds the 100 MB import limit. Copy it into the workspace using File Explorer.",
        ));
    }
    let doc: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| failure(e.to_string()))?;
    if doc["nbformat"] != 4 || !doc["cells"].is_array() {
        return Err(failure("This is not a version 4 Jupyter notebook"));
    }
    Ok(bytes)
}

pub fn import(root: &Path, source: &Path) -> AppResult<String> {
    let base = root.canonicalize().map_err(|e| failure(e.to_string()))?;
    let source = source.canonicalize().map_err(|e| failure(e.to_string()))?;
    if let Ok(relative) = source.strip_prefix(&base) {
        let relative = relative.to_string_lossy().replace('\\', "/");
        resolve(root, &relative)?;
        return Ok(relative);
    }
    let bytes = read_and_validate_notebook(&source)?;
    let file_name = source
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported Notebook.ipynb");
    let target = target_notebook_name(file_name);
    let stem = target.strip_suffix(".ipynb").unwrap_or(&target).to_owned();
    // Imported files sit beside the user's other work, without creating template folders.
    let folder = root;
    for n in 0..10000 {
        let name = if n == 0 {
            target.clone()
        } else {
            format!("{stem} ({n}).ipynb")
        };
        let path = folder.join(&name);
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
        {
            Ok(mut file) => {
                if let Err(err) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(failure(err.to_string()));
                }
                return Ok(name);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(failure(e.to_string())),
        }
    }
    Err(failure("No unused filename found"))
}

/// Sends a file to the Recycle Bin rather than deleting it outright, so a mistaken click
/// doesn't cost real work — the same safety margin Windows' own Explorer gives.
pub fn move_to_recycle_bin(path: &Path) -> AppResult<()> {
    trash::delete(path).map_err(|e| failure(e.to_string()))
}

/// Copies a downloaded notebook into a workspace, replacing whatever currently has the same
/// name.
///
/// Used only after the student has been shown what is already there and chosen to replace it
/// explicitly — nothing in SageDock overwrites a file the quiet way otherwise. The write is
/// atomic, so a failure partway through cannot leave a half-written file in place of either
/// version.
pub fn import_replacing(root: &Path, source: &Path) -> AppResult<String> {
    let base = root.canonicalize().map_err(|e| failure(e.to_string()))?;
    let source = source.canonicalize().map_err(|e| failure(e.to_string()))?;
    let bytes = read_and_validate_notebook(&source)?;
    let file_name = source
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported Notebook.ipynb");
    let target = target_notebook_name(file_name);
    crate::storage::atomic_write(&base.join(&target), &bytes)
        .map_err(|e| failure(e.to_string()))?;
    Ok(target)
}

/// Copies a file the user explicitly chose — in a native picker, or by dropping it on the
/// window — into a workspace folder, never overwriting anything already there.
///
/// Deliberately not restricted to notebooks. Coursework is datasets, images and PDFs as well
/// as `.ipynb` files, and a folder the student can already open in File Explorer gains
/// nothing from SageDock refusing to put a CSV in it. The authorization is the gesture, not
/// the extension; what this does guarantee is that nothing existing is replaced.
pub fn add_file(root: &Path, source: &Path) -> AppResult<String> {
    let base = root.canonicalize().map_err(|e| failure(e.to_string()))?;
    let source = source.canonicalize().map_err(|e| failure(e.to_string()))?;
    if !source.is_file() {
        return Err(failure("Only files can be added to a workspace"));
    }
    // Already inside this workspace: copying would hand the user a duplicate of their own
    // file for no reason.
    if let Ok(relative) = source.strip_prefix(&base) {
        return Ok(relative.to_string_lossy().replace('\\', "/"));
    }

    let name = source
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| failure("That file's name can't be read"))?;
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, Some(extension)),
        _ => (name, None),
    };

    for n in 0..10_000 {
        let candidate = match (n, extension) {
            (0, Some(extension)) => format!("{stem}.{extension}"),
            (0, None) => stem.to_owned(),
            (n, Some(extension)) => format!("{stem} ({n}).{extension}"),
            (n, None) => format!("{stem} ({n})"),
        };
        let destination = base.join(&candidate);
        // `create_new` is what makes "never overwrite" true rather than merely likely: a
        // check followed by a copy leaves a window in which something else creates the file.
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&destination)
        {
            Ok(mut file) => {
                let copied = fs::File::open(&source)
                    .and_then(|mut input| std::io::copy(&mut input, &mut file))
                    .and_then(|_| file.sync_all());
                if let Err(err) = copied {
                    drop(file);
                    // Remove the half-written copy rather than leaving a truncated file
                    // that looks like the user's data.
                    let _ = fs::remove_file(&destination);
                    return Err(failure(err.to_string()));
                }
                return Ok(candidate);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(failure(e.to_string())),
        }
    }
    Err(failure("No unused filename found"))
}

/// Notebooks sitting in the user's Downloads folder.
///
/// Shallow by design: Downloads is often enormous, and a deep walk on every Home render
/// would cost far more than the feature is worth. `path` carries the bare file name rather
/// than a full path, so no absolute path ever reaches the webview — `downloaded_file`
/// resolves it back here.
pub fn downloaded(dir: &Path, tracked: &[PathBuf]) -> AppResult<Vec<NotebookEntry>> {
    let mut entries = Vec::new();
    // Canonical paths the folder scan already listed, so a tracked download that was saved
    // into Downloads is not listed a second time.
    let mut seen: Vec<PathBuf> = Vec::new();
    // Downloads folders get large, and a notebook that sorts late alphabetically must not
    // fall off the end of the scan before it is ever considered.
    let mut budget = 5000usize;
    // An unavailable Downloads folder skips the scan but must not hide downloads the
    // built-in browser recorded, which live wherever the student chose to save them.
    let listing = if dir.is_dir() {
        Some(fs::read_dir(dir).map_err(|e| failure(e.to_string()))?)
    } else {
        None
    };
    for entry in listing.into_iter().flatten() {
        if budget == 0 {
            break;
        }
        budget -= 1;
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        // Includes Windows junctions, which must not redirect this scan somewhere else.
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 || !metadata.is_file() {
            continue;
        }
        // A notebook is JSON, and Jupyter serves `.ipynb` as `application/json`, so a
        // browser commonly saves one as `Name.json` or `Name.ipynb.json`. Matching only
        // `.ipynb` made exactly those downloads invisible here.
        let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if extension != "ipynb" && extension != "json" {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        // A Downloads folder is full of unrelated JSON — settings, API dumps, exports —
        // and listing those as notebooks would be noise. Only the head of the file is
        // read, because this runs on every Home render and some JSON is enormous.
        if extension == "json" {
            let Ok(file) = fs::File::open(&path) else {
                continue;
            };
            let mut head = Vec::new();
            if file.take(64 * 1024).read_to_end(&mut head).is_err() {
                continue;
            }
            if !String::from_utf8_lossy(&head).contains("\"nbformat\"") {
                continue;
            }
        }
        // `Report.ipynb.json` should read as "Report", not "Report.ipynb". Computed the same
        // way the copy-in path names the file, so the list and the result always agree.
        let target = target_notebook_name(name);
        let display = target.strip_suffix(".ipynb").unwrap_or(&target);
        entries.push(NotebookEntry {
            path: name.to_owned(),
            name: display.to_owned(),
            modified: metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0),
            // Already in Downloads, so the row has no other folder worth naming.
            folder: None,
            notebook: true,
        });
        seen.push(path.canonicalize().unwrap_or(path));
    }

    // Downloads the built-in browser made, wherever they were saved. These are recorded
    // rather than discovered — see `downloads.rs` — so they are listed whatever their file
    // type, while the folder scan above stays notebooks-only.
    let downloads_root = dir.canonicalize().ok();
    for path in tracked {
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        if !canonical.is_file() || seen.contains(&canonical) {
            continue;
        }
        let Some(file_name) = canonical.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let notebook = is_notebook_file(&canonical);
        let display = if notebook {
            let target = target_notebook_name(file_name);
            target.strip_suffix(".ipynb").unwrap_or(&target).to_owned()
        } else {
            file_name.to_owned()
        };
        // The containing folder's name, never its path. One saved into Downloads itself
        // reads as an ordinary download and needs no folder at all.
        let folder = match (canonical.parent(), downloads_root.as_deref()) {
            (Some(parent), Some(root)) if parent == root => None,
            (Some(parent), _) => parent.file_name().map(|s| s.to_string_lossy().into_owned()),
            (None, _) => None,
        };
        entries.push(NotebookEntry {
            path: tracked_key(&canonical),
            name: display,
            modified: fs::metadata(&canonical)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0),
            folder,
            notebook,
        });
        seen.push(canonical);
    }

    entries.sort_by_key(|e| std::cmp::Reverse(e.modified));
    entries.truncate(10);
    Ok(entries)
}

/// Marks a key that refers to a recorded download rather than a file in Downloads.
const TRACKED_PREFIX: &str = "tracked:";

/// A stable, opaque key for a recorded download.
///
/// The Downloads list mixes files in the Downloads folder, keyed by their bare name, with
/// downloads saved anywhere on disk. Those cannot be keyed by name, because two folders can
/// hold the same one, and must not be keyed by path, because no absolute path may reach the
/// webview. Hashing the path gives a key that is stable across restarts and reveals nothing
/// about where the file is.
fn tracked_key(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(path.to_string_lossy().as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{TRACKED_PREFIX}{hex}")
}

/// Whether a file is a Jupyter notebook, and so can be added to a workspace.
///
/// The extension alone is not enough: a browser saving a notebook commonly names it `.json`.
/// Only the head of the file is read, because a Downloads folder is full of large unrelated
/// JSON and this runs while Home is rendering.
fn is_notebook_file(path: &Path) -> bool {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "ipynb" => true,
        "json" => {
            let Ok(file) = fs::File::open(path) else {
                return false;
            };
            let mut head = Vec::new();
            if file.take(64 * 1024).read_to_end(&mut head).is_err() {
                return false;
            }
            String::from_utf8_lossy(&head).contains("\"nbformat\"")
        }
        _ => false,
    }
}

/// Resolves a key reported by `downloaded` back to a real file.
///
/// Two kinds of key, because the list has two sources: a bare file name in the Downloads
/// folder, or an opaque `tracked:` key for a download recorded elsewhere. A tracked key is
/// only ever matched against the recorded list, so it can never name a file SageDock did
/// not download itself.
pub fn resolve_download(dir: &Path, tracked: &[PathBuf], key: &str) -> AppResult<PathBuf> {
    if key.starts_with(TRACKED_PREFIX) {
        return tracked
            .iter()
            .filter_map(|path| path.canonicalize().ok())
            .find(|path| path.is_file() && tracked_key(path) == key)
            .ok_or_else(|| failure("That download is no longer where it was saved"));
    }
    downloaded_file(dir, key)
}

/// Resolves a bare name reported by `downloaded` back to a real file in Downloads.
///
/// The webview only ever sends the bare name, so anything carrying a path separator, a drive
/// letter or a traversal component is a bug or an attack and is refused either way.
fn downloaded_file(dir: &Path, name: &str) -> AppResult<PathBuf> {
    let input = Path::new(name);
    if name.is_empty()
        || name.contains(':')
        || name.contains('\0')
        || input.components().count() != 1
        || !matches!(input.components().next(), Some(Component::Normal(_)))
    {
        return Err(failure("Invalid downloaded notebook name"));
    }
    let base = dir.canonicalize().map_err(|e| failure(e.to_string()))?;
    let file = base
        .join(input)
        .canonicalize()
        .map_err(|e| failure(e.to_string()))?;
    // Must accept exactly what `downloaded` lists. A browser-saved notebook may carry
    // either extension, and refusing `.json` here would put files in the list that could
    // never be opened. `import` still checks the contents before copying anything.
    let extension = file.extension().and_then(|s| s.to_str()).unwrap_or("");
    if !file.starts_with(&base) || !file.is_file() || (extension != "ipynb" && extension != "json")
    {
        return Err(failure(
            "That notebook is no longer in your Downloads folder",
        ));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    #[test]
    fn imports_use_the_root_preserve_collisions_and_find_legacy_notebooks() {
        let root =
            std::env::temp_dir().join(format!("sagedock-import-flat-{}", std::process::id()));
        let workspace = root.join("workspace");
        let source = root.join("lesson.ipynb");
        std::fs::create_dir_all(&workspace).unwrap();
        let contents = br#"{"nbformat":4,"cells":[]}"#;
        std::fs::write(&source, contents).unwrap();
        std::fs::write(workspace.join("lesson.ipynb"), b"existing work").unwrap();
        let imported = super::import(&workspace, &source).unwrap();
        assert_eq!(imported, "lesson (1).ipynb");
        assert_eq!(std::fs::read(workspace.join(&imported)).unwrap(), contents);
        assert_eq!(
            std::fs::read(workspace.join("lesson.ipynb")).unwrap(),
            b"existing work"
        );
        assert!(!workspace.join("Notebooks").exists());
        // A folder created by an earlier SageDock release remains discoverable and usable.
        std::fs::create_dir(workspace.join("Notebooks")).unwrap();
        std::fs::write(workspace.join("Notebooks/legacy.ipynb"), contents).unwrap();
        let recent = super::recent(&workspace).unwrap();
        assert!(recent
            .iter()
            .any(|entry| entry.path == "Notebooks/legacy.ipynb"));
        assert!(recent.iter().any(|entry| entry.path == imported));
        assert_eq!(
            super::import(&workspace, &workspace.join(&imported)).unwrap(),
            imported
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Dropping or picking a file must never replace work already in the folder, and must
    /// not be limited to notebooks — coursework is datasets and images too.
    #[test]
    fn added_files_keep_any_type_and_never_overwrite() {
        let root = std::env::temp_dir().join(format!("sagedock-addfile-{}", std::process::id()));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(root.join("source")).unwrap();

        let csv = root.join("source/data.csv");
        std::fs::write(&csv, b"a,b\n1,2\n").unwrap();
        std::fs::write(workspace.join("data.csv"), b"EXISTING WORK").unwrap();

        let landed = super::add_file(&workspace, &csv).unwrap();
        assert_eq!(
            landed, "data (1).csv",
            "must not overwrite the existing file"
        );
        assert_eq!(
            std::fs::read(workspace.join("data.csv")).unwrap(),
            b"EXISTING WORK"
        );
        assert_eq!(
            std::fs::read(workspace.join(landed)).unwrap(),
            b"a,b\n1,2\n"
        );

        // A file with no extension still gets a unique name rather than clobbering.
        let plain = root.join("source/NOTES");
        std::fs::write(&plain, b"one").unwrap();
        assert_eq!(super::add_file(&workspace, &plain).unwrap(), "NOTES");
        std::fs::write(&plain, b"two").unwrap();
        assert_eq!(super::add_file(&workspace, &plain).unwrap(), "NOTES (1)");

        // A file already inside the workspace is left alone rather than duplicated.
        assert_eq!(
            super::add_file(&workspace, &workspace.join("data.csv")).unwrap(),
            "data.csv"
        );
        assert!(super::add_file(&workspace, &root.join("source")).is_err());

        std::fs::remove_dir_all(root).unwrap();
    }

    /// The Downloads list reports bare file names, never paths, and only notebooks.
    #[test]
    fn downloads_include_json_notebooks_but_not_other_json() {
        let dir = std::env::temp_dir().join(format!("sagedock-downloads-{}", std::process::id()));
        let nested = dir.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let notebook = br#"{"nbformat":4,"cells":[]}"#;
        std::fs::write(dir.join("Assignment 1.ipynb"), notebook).unwrap();
        // What a browser actually writes when it saves a notebook.
        std::fs::write(dir.join("Week 02 Lab.ipynb.json"), notebook).unwrap();
        std::fs::write(dir.join("Seminar.json"), notebook).unwrap();
        // Ordinary JSON, which must not be presented as coursework.
        std::fs::write(dir.join("settings.json"), br#"{"theme":"dark"}"#).unwrap();
        std::fs::write(dir.join("installer.exe"), b"nope").unwrap();
        std::fs::write(dir.join(".hidden.ipynb"), notebook).unwrap();
        std::fs::write(nested.join("deep.ipynb"), notebook).unwrap();

        let found = super::downloaded(&dir, &[]).unwrap();
        // Sorted: entries come back newest-first, and a temp directory writes them all
        // inside the same second, so their relative order is not meaningful here.
        let mut names: Vec<_> = found.iter().map(|e| e.path.as_str()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "Assignment 1.ipynb",
                "Seminar.json",
                "Week 02 Lab.ipynb.json"
            ],
            "a JSON-saved notebook must be listed, and ordinary JSON must not"
        );

        let display = |path: &str| {
            found
                .iter()
                .find(|e| e.path == path)
                .unwrap_or_else(|| panic!("{path} missing"))
                .name
                .clone()
        };
        assert_eq!(display("Week 02 Lab.ipynb.json"), "Week 02 Lab");
        assert_eq!(display("Seminar.json"), "Seminar");
        assert_eq!(display("Assignment 1.ipynb"), "Assignment 1");
        assert!(
            found
                .iter()
                .all(|e| !e.path.contains('\\') && !e.path.contains('/')),
            "the webview must never receive a path"
        );

        assert!(super::downloaded_file(&dir, "Assignment 1.ipynb").is_ok());
        assert!(super::downloaded_file(&dir, "Week 02 Lab.ipynb.json").is_ok());
        for bad in [
            "../secret.ipynb",
            "nested/deep.ipynb",
            r"nested\deep.ipynb",
            "C:\\a.ipynb",
            "installer.exe",
            "",
        ] {
            assert!(
                super::downloaded_file(&dir, bad).is_err(),
                "{bad} should be refused"
            );
        }

        // Copying one in must not produce `Week 02 Lab.ipynb.ipynb`.
        let workspace = dir.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        assert_eq!(
            super::import(&workspace, &dir.join("Week 02 Lab.ipynb.json")).unwrap(),
            "Week 02 Lab.ipynb"
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn target_notebook_name_collapses_the_json_and_ipynb_variants() {
        for (input, expected) in [
            ("Report.ipynb", "Report.ipynb"),
            ("Report.json", "Report.ipynb"),
            ("Report.ipynb.json", "Report.ipynb"),
            ("Two.Dots.ipynb", "Two.Dots.ipynb"),
        ] {
            assert_eq!(
                super::target_notebook_name(input),
                expected,
                "input was {input}"
            );
        }
    }

    /// The whole point of `import_replacing`: it is reached only after the student has
    /// explicitly chosen to overwrite, so this asserts the old content is actually gone
    /// rather than merely that the call succeeds.
    #[test]
    fn import_replacing_overwrites_the_existing_file_and_keeps_its_name() {
        let dir = std::env::temp_dir().join(format!("sagedock-replace-{}", std::process::id()));
        let workspace = dir.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("Assignment 1.ipynb"), b"OLD CONTENT").unwrap();
        let source = dir.join("Assignment 1.json");
        let fresh = br#"{"nbformat":4,"cells":["new"]}"#;
        std::fs::write(&source, fresh).unwrap();

        let landed = super::import_replacing(&workspace, &source).unwrap();
        assert_eq!(
            landed, "Assignment 1.ipynb",
            "replace must keep the original name"
        );
        assert_eq!(
            std::fs::read(workspace.join("Assignment 1.ipynb")).unwrap(),
            fresh,
            "the old content must actually be gone, not merely coexist with a new file"
        );

        // A junk source must still be refused before anything is touched.
        std::fs::write(workspace.join("Notes.ipynb"), b"KEEP ME").unwrap();
        std::fs::write(dir.join("Notes.json"), b"not a notebook").unwrap();
        assert!(super::import_replacing(&workspace, &dir.join("Notes.json")).is_err());
        assert_eq!(
            std::fs::read(workspace.join("Notes.ipynb")).unwrap(),
            b"KEEP ME",
            "a rejected replacement must not touch the existing file"
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn traversal_and_absolute_paths_are_rejected() {
        for path in [
            "../a.ipynb",
            "C:\\a.ipynb",
            "/tmp/a.ipynb",
            "a.ipynb:stream",
        ] {
            assert!(super::resolve(std::path::Path::new("."), path).is_err());
        }
    }
}
