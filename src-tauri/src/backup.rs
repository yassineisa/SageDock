//! Portable backup and restore of the user's own work.
//!
//! This is emphatically **not** the runtime backup in `runtime::repair`. That one snapshots
//! the disposable Linux environment and is tied to this machine's WSL registration. This
//! one contains coursework, and its whole purpose is to survive things: a reinstall, a new
//! Windows account, a different computer entirely.
//!
//! So a backup deliberately records nothing about where it came from. No absolute paths, no
//! Windows username, no WSL distribution, no app configuration directory. Just workspace
//! names, the files inside them, portable preferences, and a manifest describing all of it.
//! A later SageDock reads the manifest and rebuilds the workspaces wherever it happens to
//! live now.
//!
//! # Format
//!
//! An ordinary ZIP file, openable in File Explorer without SageDock, which matters when
//! somebody needs one file back and does not have the app to hand.
//!
//! ```text
//! sagedock-backup.json           the manifest (below)
//! workspaces/00-calculus/…       one folder per workspace, original tree preserved
//! workspaces/01-physics/…
//! ```
//!
//! The manifest is `format` 1 and carries the app version, creation time, per-workspace
//! file counts and sizes, portable preferences, and a SHA-256 for every file. Restoring
//! checks those hashes, which is what makes "verified" mean something rather than "the
//! write returned success".
//!
//! # Safety posture when reading
//!
//! Every archive is treated as hostile, because one can arrive by email. Entry paths are
//! resolved against the destination and rejected if they escape it, name a drive, or
//! contain `..`. Symbolic links are refused outright. Total and per-file sizes are capped
//! so a small archive cannot expand into a full disk.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};
use crate::workspaces::{self, WorkspaceStore};

/// Bumped only when the archive layout changes incompatibly.
pub const BACKUP_FORMAT: u32 = 1;

const MANIFEST_NAME: &str = "sagedock-backup.json";
const WORKSPACE_PREFIX: &str = "workspaces/";

/// Written alongside the destination and renamed into place only once verified, so an
/// interrupted backup never leaves a plausible-looking but incomplete file behind.
const PARTIAL_SUFFIX: &str = ".sagedock-part";

/// Caps that keep a hostile or accidental archive from exhausting the disk.
const MAX_TOTAL_BYTES: u64 = 96 * 1024 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_FILE_COUNT: u64 = 400_000;
const MAX_MANIFEST_BYTES: u64 = 128 * 1024 * 1024;

/// Names never worth carrying between machines: caches, build droppings, our own probe
/// files, and anything holding a Jupyter token. Notebook checkpoints are *not* here,
/// `.ipynb_checkpoints` is recovered coursework and is deliberately included.
fn is_excluded(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == ".jupyter"
        || lower == "__pycache__"
        || lower == "node_modules"
        || lower == ".venv"
        || lower == "venv"
        || lower == ".ds_store"
        || lower == "thumbs.db"
        || lower.ends_with(".pyc")
        || lower.ends_with(".tmp")
        || lower.starts_with(".sagedock-write-test-")
        || lower.starts_with(".sagedock-")
}

// --- manifest ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupFile {
    /// Archive-relative, forward slashes, never absolute.
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupWorkspace {
    /// Stable folder name inside the archive. Index-prefixed so two workspaces with the
    /// same display name cannot collide.
    pub slug: String,
    pub name: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub files: Vec<BackupFile>,
}

/// Settings worth carrying to another machine. Anything machine-specific, paths, ports,
/// tokens, the selected package, is deliberately absent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackupPreferences {
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub open_in_browser: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub format: u32,
    pub app_version: String,
    pub created_utc: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub workspaces: Vec<BackupWorkspace>,
    #[serde(default)]
    pub preferences: BackupPreferences,
}

// --- progress and results ------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct BackupProgress {
    pub stage: String,
    pub detail: Option<String>,
    pub percent: Option<f32>,
}

impl BackupProgress {
    fn new(stage: &str, detail: Option<String>, percent: Option<f32>) -> Self {
        Self {
            stage: stage.to_string(),
            detail,
            percent,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupSummary {
    pub path: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub workspaces: Vec<String>,
    /// Workspaces whose folder could not be read, named so the user can tell what is
    /// missing from the backup rather than discovering it at restore time.
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewWorkspace {
    pub name: String,
    pub file_count: u64,
    pub total_bytes: u64,
    /// Whether a workspace of this name already exists, so the UI can say in advance that
    /// it will be restored alongside rather than over it.
    pub conflicts: bool,
    pub restored_as: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupPreview {
    pub format: u32,
    pub app_version: String,
    pub created_utc: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub workspaces: Vec<PreviewWorkspace>,
    pub compatible: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RestoreSummary {
    pub restored: Vec<String>,
    pub file_count: u64,
}

// --- creating a backup -----------------------------------------------------------------------

struct Candidate {
    absolute: PathBuf,
    entry: String,
    size: u64,
    modified: Option<std::time::SystemTime>,
}

/// Walks a workspace, collecting the files worth backing up.
///
/// Links and unknown reparse points fail the operation rather than copying unrelated
/// files or silently omitting coursework. Windows cloud placeholders are read normally.
fn collect(root: &Path, slug: &str) -> AppResult<Vec<Candidate>> {
    fn walk(
        root: &Path,
        dir: &Path,
        slug: &str,
        depth: usize,
        out: &mut Vec<Candidate>,
    ) -> std::io::Result<()> {
        if depth > 32 || out.len() as u64 > MAX_FILE_COUNT {
            return Err(std::io::Error::other(
                "Workspace exceeds the backup depth or file count limit",
            ));
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_excluded(&name) {
                continue;
            }
            let metadata = std::fs::symlink_metadata(entry.path())?;
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 && !is_cloud_placeholder(&entry.path())? {
                return Err(std::io::Error::other("Workspace contains a link or unsupported reparse point. Copy its files into an ordinary local folder before backing up."));
            }
            if metadata.is_dir() {
                walk(root, &entry.path(), slug, depth + 1, out)?;
            } else if metadata.is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .unwrap_or(&entry.path())
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(Candidate {
                    absolute: entry.path(),
                    entry: format!("{WORKSPACE_PREFIX}{slug}/{relative}"),
                    size: metadata.len(),
                    modified: metadata.modified().ok(),
                });
            }
        }
        Ok(())
    }

    let mut out = Vec::new();
    walk(root, root, slug, 0, &mut out).map_err(|err| {
        AppError::new(
            "backup",
            "BACKUP_READ_FAILED",
            "SageDock couldn't read one of your workspaces",
            "A workspace couldn't be fully read. Make cloud files available offline and copy any linked folders into ordinary folders, then try again. Nothing has been written yet.",
        )
        .with_technical_details(format!("{}: {err}", root.display()))
    })?;
    Ok(out)
}

fn slugify(name: &str, index: usize) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-').to_string();
    let base = if trimmed.is_empty() {
        "workspace".to_string()
    } else {
        trimmed
    };
    format!("{index:02}-{}", base.chars().take(40).collect::<String>())
}

/// Writes a verified backup to `destination`.
///
/// The file is built beside the destination and only renamed into place after every hash in
/// its own manifest has been read back and confirmed, so a backup that exists is a backup
/// that was checked.
pub fn create(
    store: &WorkspaceStore,
    config: &crate::config::AppConfig,
    destination: &Path,
    on_progress: &mut dyn FnMut(BackupProgress),
) -> AppResult<BackupSummary> {
    on_progress(BackupProgress::new(
        "Looking through your workspaces",
        None,
        Some(0.0),
    ));

    let mut planned: Vec<(BackupWorkspace, Vec<Candidate>)> = Vec::new();
    let mut skipped = Vec::new();

    for (index, record) in store.workspaces.iter().enumerate() {
        if let (Ok(parent), Ok(root)) = (
            destination
                .parent()
                .unwrap_or(Path::new("."))
                .canonicalize(),
            record.path.canonicalize(),
        ) {
            if parent.starts_with(root) {
                return Err(AppError::new("backup", "BACKUP_DESTINATION_IN_WORKSPACE", "Save the backup outside your workspaces",
                    "Choose a separate folder or a USB drive so the backup cannot include or replace your coursework. Nothing has been changed."));
            }
        }
        if !record.path.is_dir() {
            skipped.push(record.name.clone());
            continue;
        }
        let slug = slugify(&record.name, index);
        let candidates = collect(&record.path, &slug)?;
        let total_bytes = candidates.iter().map(|c| c.size).sum();
        planned.push((
            BackupWorkspace {
                slug,
                name: record.name.clone(),
                file_count: candidates.len() as u64,
                total_bytes,
                files: Vec::new(),
            },
            candidates,
        ));
    }

    if planned.is_empty() {
        return Err(AppError::new(
            "backup",
            "BACKUP_NOTHING_TO_SAVE",
            "There's nothing to back up yet",
            "None of your workspace folders could be found. Check that they haven't been moved or renamed, then try again.",
        ));
    }

    let total_bytes: u64 = planned.iter().map(|(w, _)| w.total_bytes).sum();
    let total_files: u64 = planned.iter().map(|(w, _)| w.file_count).sum();
    if total_bytes > MAX_TOTAL_BYTES || total_files > MAX_FILE_COUNT {
        return Err(AppError::new(
            "backup",
            "BACKUP_TOO_LARGE",
            "That's more than SageDock can put in one backup",
            "Your workspaces contain a very large amount of data. Back up fewer workspaces at a time, or copy the folders directly in File Explorer.",
        ));
    }

    if !skipped.is_empty() {
        return Err(AppError::new("backup", "BACKUP_INCOMPLETE_SOURCES", "Some workspace folders are unavailable",
            format!("Reconnect these folders or remove them from the Home list, then try again: {}. Your existing files and backups have not been changed.", skipped.join(", "))));
    }

    let partial =
        partial_path(&destination.with_file_name(format!(".sagedock-{}", random_suffix()?)));

    let result = write_archive(&partial, &mut planned, config, total_bytes, on_progress);
    let manifest = match result {
        Ok(manifest) => manifest,
        Err(err) => {
            let _ = std::fs::remove_file(&partial);
            return Err(err);
        }
    };

    on_progress(BackupProgress::new(
        "Checking the backup",
        Some("Confirming every file was written correctly.".into()),
        Some(0.95),
    ));
    if let Err(err) = verify_archive(&partial, &manifest) {
        let _ = std::fs::remove_file(&partial);
        return Err(err);
    }

    // Catch files added, removed, or saved after their individual copy finished.
    let unchanged = (|| -> AppResult<bool> {
        for (index, record) in store.workspaces.iter().enumerate() {
            let fresh = collect(&record.path, &slugify(&record.name, index))?;
            let before = &planned[index].1;
            let original: std::collections::HashMap<_, _> = before
                .iter()
                .map(|c| (&c.entry, (c.size, c.modified)))
                .collect();
            if fresh.len() != before.len()
                || fresh
                    .iter()
                    .any(|c| original.get(&c.entry) != Some(&(c.size, c.modified)))
            {
                return Ok(false);
            }
        }
        Ok(true)
    })();
    if !matches!(unchanged, Ok(true)) {
        let _ = std::fs::remove_file(&partial);
        return Err(write_error(
            "Workspace files changed during backup. Save and close them, then try again.".into(),
        ));
    }

    // Only now does the file take the name the user chose.
    std::fs::rename(&partial, destination).map_err(|err| {
        let _ = std::fs::remove_file(&partial);
        write_error(format!(
            "rename {} -> {}: {err}",
            partial.display(),
            destination.display()
        ))
    })?;

    on_progress(BackupProgress::new("Backup complete", None, Some(1.0)));
    tracing::info!(target: "backup", files = manifest.file_count, "backup written and verified");

    Ok(BackupSummary {
        path: destination.display().to_string(),
        file_count: manifest.file_count,
        total_bytes: manifest.total_bytes,
        workspaces: manifest.workspaces.iter().map(|w| w.name.clone()).collect(),
        skipped,
    })
}

fn write_archive(
    partial: &Path,
    planned: &mut [(BackupWorkspace, Vec<Candidate>)],
    config: &crate::config::AppConfig,
    total_bytes: u64,
    on_progress: &mut dyn FnMut(BackupProgress),
) -> AppResult<BackupManifest> {
    use zip::write::SimpleFileOptions;

    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(partial)
        .map_err(|e| write_error(e.to_string()))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));

    let mut changed: Vec<String> = Vec::new();
    let mut done_bytes: u64 = 0;
    let mut last_percent = u64::MAX;
    let mut buffer = vec![0u8; 256 * 1024];

    for (workspace, candidates) in planned.iter_mut() {
        for candidate in candidates.iter() {
            if candidate.size > MAX_FILE_BYTES {
                return Err(AppError::new(
                    "backup",
                    "BACKUP_FILE_TOO_LARGE",
                    "One of your files is too large to back up",
                    "A single file in your workspace is larger than SageDock can include. Copy it separately in File Explorer, then back up again.",
                )
                .with_technical_details(candidate.entry.clone()));
            }

            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .large_file(candidate.size >= u32::MAX as u64);

            zip.start_file(candidate.entry.clone(), options)
                .map_err(|e| write_error(e.to_string()))?;

            let mut source = match std::fs::File::open(&candidate.absolute) {
                Ok(file) => file,
                // A file that vanished mid-backup is exactly the inconsistency this is
                // meant to detect, not something to quietly omit.
                Err(err) => {
                    changed.push(format!("{} ({err})", candidate.entry));
                    continue;
                }
            };

            let mut hasher = Sha256::new();
            let mut written: u64 = 0;
            loop {
                let read = source
                    .read(&mut buffer)
                    .map_err(|e| write_error(e.to_string()))?;
                if read == 0 {
                    break;
                }
                if written + read as u64 > candidate.size {
                    return Err(write_error("A source file grew during backup. Save and close your notebooks, then try again.".into()));
                }
                hasher.update(&buffer[..read]);
                zip.write_all(&buffer[..read])
                    .map_err(|e| write_error(e.to_string()))?;
                written += read as u64;

                done_bytes += read as u64;
                let percent = done_bytes * 90 / total_bytes.max(1);
                if percent != last_percent {
                    last_percent = percent;
                    on_progress(BackupProgress::new(
                        "Copying your files",
                        Some(workspace.name.clone()),
                        Some(done_bytes as f32 / total_bytes.max(1) as f32 * 0.9),
                    ));
                }
            }

            // Re-stat after reading. A notebook saved while the backup was running would
            // otherwise be captured half-old and half-new, and nothing would say so.
            let after = std::fs::symlink_metadata(&candidate.absolute).ok();
            let size_changed = after.as_ref().map(|m| m.len()) != Some(candidate.size)
                || written != candidate.size;
            let time_changed = after
                .as_ref()
                .and_then(|m| m.modified().ok())
                .zip(candidate.modified)
                .is_some_and(|(now, before)| now != before);
            if size_changed || time_changed {
                changed.push(candidate.entry.clone());
                continue;
            }

            workspace.files.push(BackupFile {
                path: candidate.entry.clone(),
                size: written,
                sha256: hex(&hasher.finalize()),
            });
        }
        workspace.file_count = workspace.files.len() as u64;
        workspace.total_bytes = workspace.files.iter().map(|f| f.size).sum();
    }

    if !changed.is_empty() {
        let shown: Vec<&String> = changed.iter().take(5).collect();
        return Err(AppError::new(
            "backup",
            "BACKUP_FILES_CHANGED",
            "Some files changed while the backup was being made",
            "SageDock stopped rather than save a copy that might be half-old and half-new. Save and close your open notebooks, then create the backup again. Nothing has been written.",
        )
        .with_technical_details(format!(
            "{} file(s) changed during the backup:\n{}",
            changed.len(),
            shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n")
        )));
    }

    let manifest = BackupManifest {
        format: BACKUP_FORMAT,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        created_utc: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
        file_count: planned.iter().map(|(w, _)| w.file_count).sum(),
        total_bytes: planned.iter().map(|(w, _)| w.total_bytes).sum(),
        workspaces: planned.iter().map(|(w, _)| w.clone()).collect(),
        preferences: BackupPreferences {
            theme: Some(
                serde_json::to_value(config.theme)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "system".into()),
            ),
            open_in_browser: Some(config.open_in_browser),
        },
    };

    let json = serde_json::to_vec_pretty(&manifest).map_err(|e| write_error(e.to_string()))?;
    if json.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(write_error(
            "The backup file list exceeds the supported size.".into(),
        ));
    }
    zip.start_file(
        MANIFEST_NAME,
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
    )
    .map_err(|e| write_error(e.to_string()))?;
    zip.write_all(&json)
        .map_err(|e| write_error(e.to_string()))?;

    let inner = zip.finish().map_err(|e| write_error(e.to_string()))?;
    inner
        .into_inner()
        .map_err(|e| write_error(e.to_string()))?
        .sync_all()
        .map_err(|e| write_error(e.to_string()))?;

    Ok(manifest)
}

/// Reads the finished archive back and confirms every declared hash.
fn verify_archive(path: &Path, manifest: &BackupManifest) -> AppResult<()> {
    let file = std::fs::File::open(path).map_err(|e| verify_error(e.to_string()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| verify_error(e.to_string()))?;

    let mut checked: u64 = 0;
    for workspace in &manifest.workspaces {
        for declared in &workspace.files {
            let mut entry = archive
                .by_name(&declared.path)
                .map_err(|e| verify_error(format!("{}: {e}", declared.path)))?;
            let mut hasher = Sha256::new();
            let mut buffer = vec![0u8; 256 * 1024];
            let mut size: u64 = 0;
            loop {
                let read = entry
                    .read(&mut buffer)
                    .map_err(|e| verify_error(e.to_string()))?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
                size += read as u64;
            }
            if size != declared.size || hex(&hasher.finalize()) != declared.sha256 {
                return Err(verify_error(format!(
                    "{} did not match its checksum",
                    declared.path
                )));
            }
            checked += 1;
        }
    }

    if checked != manifest.file_count {
        return Err(verify_error(format!(
            "manifest declares {} files, archive contains {checked}",
            manifest.file_count
        )));
    }
    Ok(())
}

// --- reading a backup -------------------------------------------------------------------------

fn read_manifest(path: &Path) -> AppResult<BackupManifest> {
    let file = std::fs::File::open(path).map_err(|e| unreadable_error(e.to_string()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| unreadable_error(e.to_string()))?;
    let entry = archive
        .by_name(MANIFEST_NAME)
        .map_err(|_| not_a_backup_error())?;
    let mut raw = String::new();
    if entry.size() > MAX_MANIFEST_BYTES {
        return Err(unsafe_archive_error("backup manifest is too large"));
    }
    entry
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_string(&mut raw)
        .map_err(|e| unreadable_error(e.to_string()))?;
    if raw.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(unsafe_archive_error(
            "backup manifest expanded beyond its limit",
        ));
    }
    serde_json::from_str(&raw).map_err(|e| unreadable_error(e.to_string()))
}

/// Describes what a backup holds without changing anything on disk.
#[cfg(test)]
pub fn preview(path: &Path, store: &WorkspaceStore) -> AppResult<BackupPreview> {
    preview_in(path, store, None)
}

pub fn preview_in(
    path: &Path,
    store: &WorkspaceStore,
    parent: Option<&Path>,
) -> AppResult<BackupPreview> {
    let manifest = read_manifest(path)?;
    let compatible = manifest.format == BACKUP_FORMAT;
    if compatible {
        validate_manifest(&manifest)?;
    }

    let mut taken: Vec<String> = store.workspaces.iter().map(|w| w.name.clone()).collect();
    if let Some(parent) = parent.filter(|p| p.exists()) {
        for entry in std::fs::read_dir(parent).map_err(|e| unreadable_error(e.to_string()))? {
            taken.push(
                entry
                    .map_err(|e| unreadable_error(e.to_string()))?
                    .file_name()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    let workspaces = manifest
        .workspaces
        .iter()
        .map(|w| {
            let conflicts = taken.iter().any(|name| name.eq_ignore_ascii_case(&w.name));
            let restored_as = unique_name(&w.name, &taken);
            taken.push(restored_as.clone());
            PreviewWorkspace {
                name: w.name.clone(),
                file_count: w.file_count,
                total_bytes: w.total_bytes,
                conflicts,
                restored_as,
            }
        })
        .collect();

    Ok(BackupPreview {
        format: manifest.format,
        app_version: manifest.app_version.clone(),
        created_utc: manifest.created_utc.clone(),
        file_count: manifest.file_count,
        total_bytes: manifest.total_bytes,
        workspaces,
        compatible,
        note: (!compatible).then(|| {
            "This backup was made by a newer version of SageDock. Update SageDock, then restore it.".to_string()
        }),
    })
}

/// Picks a free workspace name, never one already in use.
fn unique_name(name: &str, taken: &[String]) -> String {
    let name = workspaces::validate_name(name).unwrap_or_else(|_| "Restored workspace".into());
    let name = name.as_str();
    let free = |candidate: &str| !taken.iter().any(|t| t.eq_ignore_ascii_case(candidate));
    if free(name) {
        return name.to_string();
    }
    let base: String = name.chars().take(42).collect();
    let restored = format!("{base} (restored)");
    if free(&restored) {
        return restored;
    }
    for n in 2..=taken.len() + 2 {
        let candidate = format!("{base} (restored {n})");
        if free(&candidate) {
            return candidate;
        }
    }
    unreachable!("more candidates than taken names")
}

/// Resolves an archive entry to a path inside `root`, or refuses it.
///
/// This is the function standing between a downloaded file and the rest of the disk, so it
/// refuses anything it does not positively understand rather than trying to sanitize it:
/// absolute paths, drive letters, `..`, backslashes (ZIP mandates forward slashes, so a
/// backslash means someone is being clever), and NUL bytes.
pub fn safe_destination(root: &Path, entry: &str) -> Option<PathBuf> {
    if entry.is_empty()
        || entry.contains('\0')
        || entry.contains('\\')
        || entry.starts_with('/')
        || entry.contains(':')
    {
        return None;
    }

    let mut resolved = root.to_path_buf();
    for part in entry.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part.chars().any(|c| c < ' ' || "<>:\"|?*".contains(c))
            || reserved_component(part)
        {
            return None;
        }
        // Rejects anything Windows would reinterpret, including a component that is itself
        // an absolute path or a stream reference.
        if Path::new(part).components().count() != 1 {
            return None;
        }
        if !matches!(
            Path::new(part).components().next(),
            Some(Component::Normal(_))
        ) {
            return None;
        }
        resolved.push(part);
    }

    // Belt and braces: the assembled path must still be under the root.
    resolved.starts_with(root).then_some(resolved)
}

/// Restores every workspace in a backup, never overwriting existing coursework.
#[cfg(test)]
pub fn restore(
    path: &Path,
    parent: &Path,
    store: &mut WorkspaceStore,
    on_progress: &mut dyn FnMut(BackupProgress),
) -> AppResult<RestoreSummary> {
    restore_with_commit(path, parent, store, on_progress, &mut |_| Ok(()))
}

/// Publish only after every workspace validates; registration is part of the transaction.
pub fn restore_with_commit(
    path: &Path,
    parent: &Path,
    store: &mut WorkspaceStore,
    on_progress: &mut dyn FnMut(BackupProgress),
    commit: &mut dyn FnMut(&WorkspaceStore) -> AppResult<()>,
) -> AppResult<RestoreSummary> {
    let manifest = read_manifest(path)?;
    if manifest.format > BACKUP_FORMAT {
        return Err(AppError::new(
            "backup",
            "BACKUP_TOO_NEW",
            "This backup needs a newer SageDock",
            "The backup was made by a newer version of SageDock. Update SageDock, then restore it. The backup file has not been changed.",
        ));
    }
    validate_manifest(&manifest)?;

    // Room for the files plus a margin, checked before a single byte is written.
    let needed_gib = (manifest.total_bytes / (1024 * 1024 * 1024)) + 2;
    crate::runtime::health::require_space(parent, needed_gib)?;

    on_progress(BackupProgress::new("Opening the backup", None, Some(0.0)));
    let file = std::fs::File::open(path).map_err(|e| unreadable_error(e.to_string()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| unreadable_error(e.to_string()))?;

    let mut taken: Vec<String> = store.workspaces.iter().map(|w| w.name.clone()).collect();
    for entry in std::fs::read_dir(parent).map_err(|e| restore_error(e.to_string()))? {
        taken.push(
            entry
                .map_err(|e| restore_error(e.to_string()))?
                .file_name()
                .to_string_lossy()
                .into_owned(),
        );
    }
    let mut restored_names = Vec::new();
    let mut done: u64 = 0;
    let mut candidate_store = store.clone();
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut published: Vec<PathBuf> = Vec::new();
    let outcome = (|| -> AppResult<()> {
        for workspace in &manifest.workspaces {
            let validated = unique_name(&workspace.name, &taken);
            taken.push(validated.clone());

            let final_dir = parent.join(&validated);
            // Extract into a staging folder alongside, so an interrupted restore cannot be
            // mistaken for a complete workspace.
            let staging = parent.join(format!(".sagedock-restore-{}", random_suffix()?));
            std::fs::create_dir(&staging).map_err(|e| restore_error(e.to_string()))?;
            staged.push((staging.clone(), final_dir));
            extract_workspace(
                &mut archive,
                workspace,
                &staging,
                &mut done,
                manifest.file_count,
                on_progress,
            )?;
            restored_names.push(validated);
        }
        for (staging, final_dir) in &staged {
            if std::fs::symlink_metadata(final_dir).is_ok() {
                return Err(restore_error(format!(
                    "{} already exists",
                    final_dir.display()
                )));
            }
            // On Windows rename refuses an existing directory, including one created after
            // the check above. Only directories this transaction published are rolled back.
            std::fs::rename(staging, final_dir).map_err(|e| restore_error(e.to_string()))?;
            published.push(final_dir.clone());
            workspaces::add_existing(&mut candidate_store, final_dir)?;
        }
        commit(&candidate_store)?;
        Ok(())
    })();
    if let Err(err) = outcome {
        for path in published.iter().chain(staged.iter().map(|(p, _)| p)) {
            if let Err(cleanup) = std::fs::remove_dir_all(path) {
                if cleanup.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(target: "backup", error = %cleanup, "restore cleanup failed; original backup is intact");
                }
            }
        }
        return Err(err);
    }
    *store = candidate_store;

    on_progress(BackupProgress::new("Restore complete", None, Some(1.0)));
    tracing::info!(target: "backup", count = restored_names.len(), "backup restored");
    Ok(RestoreSummary {
        restored: restored_names,
        file_count: manifest.file_count,
    })
}

fn extract_workspace(
    archive: &mut zip::ZipArchive<std::fs::File>,
    workspace: &BackupWorkspace,
    staging: &Path,
    done: &mut u64,
    total: u64,
    on_progress: &mut dyn FnMut(BackupProgress),
) -> AppResult<()> {
    let prefix = format!("{WORKSPACE_PREFIX}{}/", workspace.slug);

    for declared in &workspace.files {
        let mut entry = archive
            .by_name(&declared.path)
            .map_err(|_| damaged_error(format!("{} is missing from the archive", declared.path)))?;

        // A symlink entry could point anywhere on the machine once followed.
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0xF000 == 0xA000)
        {
            return Err(unsafe_archive_error("the archive contains a symbolic link"));
        }
        if entry.size() > MAX_FILE_BYTES || entry.size() != declared.size {
            return Err(unsafe_archive_error(
                "the archive contains an unreasonably large file",
            ));
        }

        let relative = declared
            .path
            .strip_prefix(&prefix)
            .ok_or_else(|| unsafe_archive_error("an entry sits outside its workspace"))?;
        let destination = safe_destination(staging, relative).ok_or_else(|| {
            unsafe_archive_error("an entry tried to write outside the restore folder")
        })?;

        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|e| restore_error(e.to_string()))?;
        }

        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|e| restore_error(e.to_string()))?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 256 * 1024];
        let mut size: u64 = 0;
        loop {
            let read = entry
                .read(&mut buffer)
                .map_err(|e| damaged_error(e.to_string()))?;
            if read == 0 {
                break;
            }
            size += read as u64;
            if size > declared.size {
                return Err(unsafe_archive_error(
                    "a file expanded beyond its declared size",
                ));
            }
            hasher.update(&buffer[..read]);
            out.write_all(&buffer[..read])
                .map_err(|e| restore_error(e.to_string()))?;
        }

        if size != declared.size || hex(&hasher.finalize()) != declared.sha256 {
            return Err(damaged_error(format!(
                "{} did not match its checksum",
                declared.path
            )));
        }
        out.sync_all().map_err(|e| restore_error(e.to_string()))?;

        *done += 1;
        on_progress(BackupProgress::new(
            "Restoring your files",
            Some(workspace.name.clone()),
            Some(*done as f32 / total.max(1) as f32),
        ));
    }
    Ok(())
}

// --- helpers -----------------------------------------------------------------------------------

fn random_suffix() -> AppResult<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| write_error(e.to_string()))?;
    Ok(hex(&bytes))
}

/// Cloud placeholders are reparse points too, but do not redirect to another directory.
/// Permit only the Windows CLOUD tag family; links and unknown providers fail closed.
fn is_cloud_placeholder(path: &Path) -> std::io::Result<bool> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FileAttributeTagInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_TAG_INFO,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    // The handle and correctly sized output structure remain live for this call.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of_val(&info) as u32,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(info.ReparseTag & !0x0000_f000 == 0x9000_001a)
}

fn reserved_component(part: &str) -> bool {
    let stem = part
        .split('.')
        .next()
        .unwrap_or(part)
        .trim_end()
        .to_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

/// Recompute limits and reject Windows path aliases before preview or extraction. The
/// manifest is untrusted input, including its aggregate sizes and workspace boundaries.
fn validate_manifest(manifest: &BackupManifest) -> AppResult<()> {
    use std::collections::HashSet;
    let bad = || {
        unsafe_archive_error(
            "the backup has inconsistent sizes, duplicate paths, or unsupported Windows filenames",
        )
    };
    if manifest.format != BACKUP_FORMAT
        || manifest.workspaces.is_empty()
        || manifest.workspaces.len() > 10_000
    {
        return Err(bad());
    }
    let mut slugs = HashSet::new();
    let mut count = 0u64;
    let mut total = 0u64;
    for workspace in &manifest.workspaces {
        if workspace.slug.contains('/')
            || safe_destination(Path::new("root"), &workspace.slug).is_none()
            || !slugs.insert(workspace.slug.to_lowercase())
        {
            return Err(bad());
        }
        let prefix = format!("{WORKSPACE_PREFIX}{}/", workspace.slug);
        let mut paths = HashSet::new();
        let mut directories = HashSet::new();
        let mut subtotal = 0u64;
        for file in &workspace.files {
            let relative = file.path.strip_prefix(&prefix).ok_or_else(bad)?;
            if safe_destination(Path::new("root"), relative).is_none()
                || file.size > MAX_FILE_BYTES
                || file.sha256.len() != 64
                || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(bad());
            }
            let key = relative.to_uppercase();
            if !paths.insert(key.clone()) {
                return Err(bad());
            }
            for (i, _) in key.match_indices('/') {
                directories.insert(key[..i].to_string());
            }
            subtotal = subtotal.checked_add(file.size).ok_or_else(bad)?;
        }
        if !paths.is_disjoint(&directories)
            || workspace.file_count != workspace.files.len() as u64
            || workspace.total_bytes != subtotal
        {
            return Err(bad());
        }
        total = total.checked_add(subtotal).ok_or_else(bad)?;
        count = count
            .checked_add(workspace.files.len() as u64)
            .ok_or_else(bad)?;
        if total > MAX_TOTAL_BYTES || count > MAX_FILE_COUNT {
            return Err(bad());
        }
    }
    if total != manifest.total_bytes || count != manifest.file_count {
        return Err(bad());
    }
    Ok(())
}

fn partial_path(destination: &Path) -> PathBuf {
    let mut name = destination.as_os_str().to_owned();
    name.push(PARTIAL_SUFFIX);
    PathBuf::from(name)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

fn write_error(details: String) -> AppError {
    AppError::new(
        "backup",
        "BACKUP_WRITE_FAILED",
        "SageDock couldn't finish writing the backup",
        "The backup wasn't completed, and the incomplete file has been removed. Check that the destination has enough free space and try again. Your notebooks are unaffected.",
    )
    .with_technical_details(details)
}

fn verify_error(details: String) -> AppError {
    AppError::new(
        "backup",
        "BACKUP_VERIFY_FAILED",
        "The backup couldn't be verified",
        "SageDock checked the backup it had just written and it didn't match. The incomplete file has been removed rather than left looking usable. Try again, ideally to a different drive.",
    )
    .with_technical_details(details)
}

fn unreadable_error(details: String) -> AppError {
    AppError::new(
        "backup",
        "BACKUP_UNREADABLE",
        "SageDock couldn't open that backup",
        "The file couldn't be read. If it's on a USB drive or a network location, copy it onto this PC first, then try again.",
    )
    .with_technical_details(details)
}

fn not_a_backup_error() -> AppError {
    AppError::new(
        "backup",
        "BACKUP_NOT_RECOGNISED",
        "That isn't a SageDock backup",
        "The file you chose doesn't contain a SageDock backup. Look for the .zip file created by Create backup. Nothing on your PC has been changed.",
    )
}

fn damaged_error(details: String) -> AppError {
    AppError::new(
        "backup",
        "BACKUP_DAMAGED",
        "That backup is incomplete or damaged",
        "The backup didn't match its own checksums, so SageDock stopped rather than restore files that may be corrupted. If you have another copy of the backup, try that one.",
    )
    .with_technical_details(details)
}

fn unsafe_archive_error(details: &str) -> AppError {
    AppError::new(
        "backup",
        "BACKUP_UNSAFE",
        "SageDock refused to open that backup",
        "The backup contains unsupported paths, sizes, or inconsistent file information. Your existing workspaces and the backup have not been changed. Try another backup or use the SageDock version that created it.",
    )
    .with_technical_details(details.to_string())
}

fn restore_error(details: String) -> AppError {
    AppError::new(
        "backup",
        "RESTORE_FAILED",
        "SageDock couldn't finish restoring that backup",
        "Your existing workspaces and the original backup have not been changed. Check available disk space and try again. If Windows prevented cleanup, an incomplete restore folder may remain.",
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
        let dir = std::env::temp_dir().join(format!("sagedock-backup-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Every one of these is a real archive attack or a Windows path quirk that would put
    /// a file somewhere the user never agreed to.
    #[test]
    fn hostile_entry_paths_are_refused() {
        let root = Path::new(r"C:\Restore");
        for entry in [
            "../escape.ipynb",
            "a/../../escape.ipynb",
            "/absolute.ipynb",
            "C:/drive.ipynb",
            "C:absolute.ipynb",
            r"back\slash.ipynb",
            "nul\0byte.ipynb",
            "",
            "./relative.ipynb",
            "a//b.ipynb",
        ] {
            assert!(
                safe_destination(root, entry).is_none(),
                "{entry:?} should be refused"
            );
        }
    }

    #[test]
    fn ordinary_entry_paths_resolve_inside_the_destination() {
        let root = Path::new(r"C:\Restore");
        let resolved = safe_destination(root, "Notebooks/Week 1/lab.ipynb").unwrap();
        assert_eq!(
            resolved,
            root.join("Notebooks").join("Week 1").join("lab.ipynb")
        );
        assert!(resolved.starts_with(root));
    }

    #[test]
    fn notebook_checkpoints_are_kept_but_caches_and_tokens_are_not() {
        assert!(
            !is_excluded(".ipynb_checkpoints"),
            "checkpoints are recovered coursework"
        );
        assert!(!is_excluded("lab.ipynb"));
        assert!(!is_excluded("data.csv"));

        for name in [
            ".jupyter",
            "__pycache__",
            "node_modules",
            "module.pyc",
            "scratch.tmp",
            "Thumbs.db",
        ] {
            assert!(is_excluded(name), "{name} should be excluded");
        }
        assert!(is_excluded(".sagedock-write-test-abc123"));
    }

    #[test]
    fn slugs_are_filesystem_safe_and_cannot_collide() {
        assert_eq!(slugify("Calculus", 0), "00-calculus");
        assert_eq!(slugify("Physics 101", 1), "01-physics-101");
        // Two workspaces sharing a display name still get distinct archive folders.
        assert_ne!(slugify("Maths", 0), slugify("Maths", 1));
        assert!(!slugify("../evil", 3).contains('/'));
        assert!(!slugify("../evil", 3).contains('.'));
    }

    #[test]
    fn a_restored_name_never_takes_an_existing_one() {
        let taken = vec!["Calculus".to_string(), "Calculus (restored)".to_string()];
        assert_eq!(unique_name("Physics", &taken), "Physics");
        assert_eq!(unique_name("Calculus", &taken), "Calculus (restored 2)");
        // Case-insensitively, because Windows treats these as the same folder.
        assert_eq!(unique_name("calculus", &taken), "calculus (restored 2)");
    }

    #[test]
    fn the_partial_file_sits_beside_the_destination() {
        let partial = partial_path(Path::new(r"D:\Backups\work.zip"));
        assert_eq!(partial, PathBuf::from(r"D:\Backups\work.zip.sagedock-part"));
    }

    /// The round trip that matters: real files in, verified archive out, same bytes back,
    /// restored alongside the original rather than over it.
    #[test]
    fn a_backup_round_trips_and_never_overwrites_existing_work() {
        let root = temp_dir();
        let parent = root.join("workspaces");
        std::fs::create_dir_all(&parent).unwrap();

        let mut store = WorkspaceStore::default();
        let source = workspaces::create(&mut store, &parent, "Calculus").unwrap();
        // Legacy nested layouts still round-trip without being flattened.
        std::fs::create_dir(source.path.join("Notebooks")).unwrap();
        std::fs::write(
            source.path.join("Notebooks").join("week1.ipynb"),
            b"{\"cells\":[]}",
        )
        .unwrap();
        std::fs::create_dir_all(source.path.join("Notebooks").join(".ipynb_checkpoints")).unwrap();
        std::fs::write(
            source
                .path
                .join("Notebooks")
                .join(".ipynb_checkpoints")
                .join("week1-checkpoint.ipynb"),
            b"checkpoint",
        )
        .unwrap();
        std::fs::write(source.path.join(".sagedock-write-test-xyz"), b"probe").unwrap();

        let destination = root.join("backup.zip");
        let summary = create(
            &store,
            &crate::config::AppConfig::default(),
            &destination,
            &mut |_| {},
        )
        .unwrap();

        assert!(
            destination.is_file(),
            "the backup must exist at the chosen name"
        );
        assert!(
            !partial_path(&destination).exists(),
            "no partial file may be left behind"
        );
        assert_eq!(
            summary.file_count, 2,
            "two notebooks, and not the probe file"
        );
        assert!(summary.skipped.is_empty());

        let preview = preview(&destination, &store).unwrap();
        assert!(preview.compatible);
        assert_eq!(preview.workspaces.len(), 1);
        assert!(
            preview.workspaces[0].conflicts,
            "the name is already in use"
        );
        assert_eq!(preview.workspaces[0].restored_as, "Calculus (restored)");

        let restored = restore(&destination, &parent, &mut store, &mut |_| {}).unwrap();
        assert_eq!(restored.restored, vec!["Calculus (restored)".to_string()]);

        // The original is untouched, and the copy has the same bytes.
        assert_eq!(
            std::fs::read(source.path.join("Notebooks").join("week1.ipynb")).unwrap(),
            b"{\"cells\":[]}"
        );
        let copy = parent
            .join("Calculus (restored)")
            .join("Notebooks")
            .join("week1.ipynb");
        assert_eq!(std::fs::read(&copy).unwrap(), b"{\"cells\":[]}");
        assert!(
            parent
                .join("Calculus (restored)")
                .join("Notebooks")
                .join(".ipynb_checkpoints")
                .join("week1-checkpoint.ipynb")
                .is_file(),
            "checkpoints must survive the round trip"
        );
        assert_eq!(
            store.workspaces.len(),
            2,
            "restoring adds a workspace, never replaces one"
        );
    }

    #[test]
    fn a_file_that_is_not_a_backup_is_rejected_clearly() {
        let dir = temp_dir();
        let bogus = dir.join("holiday-photos.zip");
        std::fs::write(&bogus, b"not a zip at all").unwrap();
        let store = WorkspaceStore::default();

        assert_eq!(
            preview(&bogus, &store).unwrap_err().code,
            "BACKUP_UNREADABLE"
        );
    }

    /// Builds an archive byte by byte, so these tests exercise the reader against exactly
    /// the bytes intended rather than whatever the compressor happened to emit. Stored
    /// (uncompressed) entries keep the archive contents predictable.
    fn write_crafted(path: &Path, manifest: &BackupManifest, entries: &[(&str, &[u8])]) {
        use zip::write::SimpleFileOptions;
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for (name, bytes) in entries {
            zip.start_file((*name).to_string(), options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.start_file(MANIFEST_NAME.to_string(), options).unwrap();
        zip.write_all(&serde_json::to_vec(manifest).unwrap())
            .unwrap();
        zip.finish().unwrap();
    }

    fn crafted_manifest(format: u32, slug: &str, files: Vec<BackupFile>) -> BackupManifest {
        BackupManifest {
            format,
            app_version: "1.1.0".into(),
            created_utc: "2026-09-17T00:00:00Z".into(),
            file_count: files.len() as u64,
            total_bytes: files.iter().map(|f| f.size).sum(),
            workspaces: vec![BackupWorkspace {
                slug: slug.into(),
                name: "Physics".into(),
                file_count: files.len() as u64,
                total_bytes: files.iter().map(|f| f.size).sum(),
                files,
            }],
            preferences: BackupPreferences::default(),
        }
    }

    /// Content that doesn't match its recorded checksum must never reach the workspace,
    /// silently restoring corrupted coursework would be worse than refusing.
    #[test]
    fn content_that_fails_its_checksum_is_refused() {
        let root = temp_dir();
        let parent = root.join("workspaces");
        std::fs::create_dir_all(&parent).unwrap();
        let archive = root.join("tampered.zip");

        let manifest = crafted_manifest(
            BACKUP_FORMAT,
            "00-physics",
            vec![BackupFile {
                path: "workspaces/00-physics/Notebooks/lab.ipynb".into(),
                size: 8,
                // The hash of "original", while the archive actually holds "TAMPERED".
                sha256: hex(&Sha256::digest(b"original")),
            }],
        );
        write_crafted(
            &archive,
            &manifest,
            &[("workspaces/00-physics/Notebooks/lab.ipynb", b"TAMPERED")],
        );

        let mut store = WorkspaceStore::default();
        let err = restore(&archive, &parent, &mut store, &mut |_| {}).unwrap_err();

        assert_eq!(err.code, "BACKUP_DAMAGED");
        assert!(
            !parent.join("Physics").exists(),
            "a failed restore must leave nothing behind"
        );
        assert!(store.workspaces.is_empty(), "nothing should be registered");
    }

    /// An entry that claims to belong to a workspace it isn't under is how an archive tries
    /// to place files somewhere the user never agreed to.
    #[test]
    fn an_entry_outside_its_workspace_is_refused() {
        let root = temp_dir();
        let parent = root.join("workspaces");
        std::fs::create_dir_all(&parent).unwrap();
        let archive = root.join("hostile.zip");

        let manifest = crafted_manifest(
            BACKUP_FORMAT,
            "00-physics",
            vec![BackupFile {
                path: "somewhere-else/escape.ipynb".into(),
                size: 4,
                sha256: hex(&Sha256::digest(b"evil")),
            }],
        );
        write_crafted(
            &archive,
            &manifest,
            &[("somewhere-else/escape.ipynb", b"evil")],
        );

        let mut store = WorkspaceStore::default();
        let err = restore(&archive, &parent, &mut store, &mut |_| {}).unwrap_err();

        assert_eq!(err.code, "BACKUP_UNSAFE");
        assert!(!root.join("escape.ipynb").exists());
        assert!(!parent.join("escape.ipynb").exists());
    }

    #[test]
    fn a_missing_entry_is_reported_as_damage_rather_than_ignored() {
        let root = temp_dir();
        let parent = root.join("workspaces");
        std::fs::create_dir_all(&parent).unwrap();
        let archive = root.join("incomplete.zip");

        // The manifest promises a file the archive does not contain.
        let manifest = crafted_manifest(
            BACKUP_FORMAT,
            "00-physics",
            vec![BackupFile {
                path: "workspaces/00-physics/Notebooks/lab.ipynb".into(),
                size: 4,
                sha256: hex(&Sha256::digest(b"gone")),
            }],
        );
        write_crafted(&archive, &manifest, &[]);

        let mut store = WorkspaceStore::default();
        assert_eq!(
            restore(&archive, &parent, &mut store, &mut |_| {})
                .unwrap_err()
                .code,
            "BACKUP_DAMAGED"
        );
    }

    #[test]
    fn a_backup_from_a_newer_sagedock_is_described_but_not_restored() {
        let root = temp_dir();
        let parent = root.join("workspaces");
        std::fs::create_dir_all(&parent).unwrap();
        let archive = root.join("future.zip");

        let manifest = crafted_manifest(BACKUP_FORMAT + 1, "00-physics", Vec::new());
        write_crafted(&archive, &manifest, &[]);

        let mut store = WorkspaceStore::default();

        // Previewing still works, so the user is told why rather than shown a dead end.
        let preview = preview(&archive, &store).unwrap();
        assert!(!preview.compatible);
        assert!(preview.note.is_some());

        assert_eq!(
            restore(&archive, &parent, &mut store, &mut |_| {})
                .unwrap_err()
                .code,
            "BACKUP_TOO_NEW"
        );
    }

    #[test]
    fn backing_up_with_no_readable_workspace_says_so() {
        let dir = temp_dir();
        let mut store = WorkspaceStore::default();
        store.workspaces.push(crate::workspaces::WorkspaceRecord {
            id: "ws-missing".into(),
            name: "Gone".into(),
            path: dir.join("not-here"),
            last_opened: None,
        });

        let err = create(
            &store,
            &crate::config::AppConfig::default(),
            &dir.join("b.zip"),
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(err.code, "BACKUP_NOTHING_TO_SAVE");
    }

    #[test]
    fn windows_aliases_and_device_paths_are_rejected() {
        for path in [
            "NUL.txt",
            "CON",
            "a/COM1.csv",
            "LPT²",
            "name.",
            "name ",
            "a?.txt",
            "CONIN$",
            "a\u{1}.txt",
        ] {
            assert!(
                safe_destination(Path::new("root"), path).is_none(),
                "accepted {path}"
            );
        }
    }

    #[test]
    fn manifest_totals_and_case_collisions_are_checked() {
        let file = BackupFile {
            path: "workspaces/course/Lab.txt".into(),
            size: 4,
            sha256: hex(&Sha256::digest(b"test")),
        };
        let mut manifest = crafted_manifest(1, "course", vec![file.clone()]);
        manifest.total_bytes = 0;
        assert!(validate_manifest(&manifest).is_err());
        let mut alias = file.clone();
        alias.path = "workspaces/course/lab.txt".into();
        assert!(
            validate_manifest(&crafted_manifest(1, "course", vec![file.clone(), alias])).is_err()
        );
        let mut child = file.clone();
        child.path.push_str("/child.txt");
        assert!(validate_manifest(&crafted_manifest(1, "course", vec![file, child])).is_err());
    }

    #[test]
    fn damaged_second_workspace_publishes_nothing_and_keeps_old_staging() {
        let root = temp_dir();
        let parent = root.join("restore");
        std::fs::create_dir_all(parent.join(".Physics.sagedock-part")).unwrap();
        let sentinel = parent.join(".Physics.sagedock-part/coursework.txt");
        std::fs::write(&sentinel, "keep").unwrap();
        let first = BackupFile {
            path: "workspaces/first/a.txt".into(),
            size: 4,
            sha256: hex(&Sha256::digest(b"good")),
        };
        let second = BackupFile {
            path: "workspaces/second/a.txt".into(),
            size: 4,
            sha256: hex(&Sha256::digest(b"good")),
        };
        let mut manifest = crafted_manifest(1, "first", vec![first]);
        let mut workspace = crafted_manifest(1, "second", vec![second])
            .workspaces
            .remove(0);
        workspace.name = "Chemistry".into();
        manifest.workspaces.push(workspace);
        manifest.file_count = 2;
        manifest.total_bytes = 8;
        let archive = root.join("bad.zip");
        write_crafted(
            &archive,
            &manifest,
            &[
                ("workspaces/first/a.txt", b"good"),
                ("workspaces/second/a.txt", b"oops"),
            ],
        );
        let mut store = WorkspaceStore::default();
        assert!(restore(&archive, &parent, &mut store, &mut |_| {}).is_err());
        assert!(store.workspaces.is_empty());
        assert_eq!(std::fs::read_to_string(sentinel).unwrap(), "keep");
        assert_eq!(std::fs::read_dir(parent).unwrap().count(), 1);
    }

    #[test]
    fn registration_failure_rolls_back_restored_folders() {
        let root = temp_dir();
        let parent = root.join("restore");
        std::fs::create_dir(&parent).unwrap();
        let archive = root.join("backup.zip");
        let manifest = crafted_manifest(
            1,
            "course",
            vec![BackupFile {
                path: "workspaces/course/a.txt".into(),
                size: 4,
                sha256: hex(&Sha256::digest(b"good")),
            }],
        );
        write_crafted(&archive, &manifest, &[("workspaces/course/a.txt", b"good")]);
        let mut store = WorkspaceStore::default();
        let result = restore_with_commit(&archive, &parent, &mut store, &mut |_| {}, &mut |_| {
            Err(restore_error("disk full".into()))
        });
        assert!(result.is_err());
        assert!(store.workspaces.is_empty());
        assert_eq!(std::fs::read_dir(parent).unwrap().count(), 0);
    }

    #[test]
    fn restore_preserves_unregistered_folders_and_bounds_long_names() {
        let root = temp_dir();
        let parent = root.join("restore");
        std::fs::create_dir_all(parent.join("Physics")).unwrap();
        std::fs::write(parent.join("Physics/keep.txt"), "keep").unwrap();
        let archive = root.join("backup.zip");
        write_crafted(&archive, &crafted_manifest(1, "course", Vec::new()), &[]);
        let mut store = WorkspaceStore::default();
        let result = restore(&archive, &parent, &mut store, &mut |_| {}).unwrap();
        assert_eq!(result.restored, ["Physics (restored)"]);
        assert_eq!(
            std::fs::read_to_string(parent.join("Physics/keep.txt")).unwrap(),
            "keep"
        );
        let long = "a".repeat(64);
        assert!(
            workspaces::validate_name(&unique_name(&long, std::slice::from_ref(&long))).is_ok()
        );
    }

    #[test]
    fn oversized_manifest_is_rejected_before_decompression() {
        let root = temp_dir();
        let path = root.join("bomb.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file(
            MANIFEST_NAME,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        let chunk = vec![b' '; 1024 * 1024];
        for _ in 0..129 {
            zip.write_all(&chunk).unwrap();
        }
        zip.finish().unwrap();
        assert_eq!(
            preview(&path, &WorkspaceStore::default()).unwrap_err().code,
            "BACKUP_UNSAFE"
        );
    }

    #[test]
    fn backup_inside_workspace_is_refused_and_existing_partial_is_preserved() {
        let root = temp_dir();
        let mut store = WorkspaceStore::default();
        let record = workspaces::create(&mut store, &root, "Physics").unwrap();
        assert!(create(
            &store,
            &crate::config::AppConfig::default(),
            &record.path.join("backup.zip"),
            &mut |_| {}
        )
        .is_err());
        let target = root.join("backup.zip");
        let old_partial = partial_path(&target);
        std::fs::write(&old_partial, "keep").unwrap();
        create(
            &store,
            &crate::config::AppConfig::default(),
            &target,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(old_partial).unwrap(), "keep");
    }

    #[test]
    fn too_deep_a_workspace_fails_instead_of_silently_omitting_files() {
        let root = temp_dir();
        let deep = (0..34).fold(root.clone(), |p, _| p.join("d"));
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("coursework.txt"), "keep").unwrap();
        assert!(collect(&root, "course").is_err());
    }
}
