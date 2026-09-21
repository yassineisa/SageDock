//! The SageMath package SageDock installs from: a prebuilt runtime image on local disk.
//!
//! SageDock does not download SageMath during setup. SageMath no longer publishes prebuilt
//! Linux binaries, and pulling its ~2 GB of packages on every student's machine proved
//! unreliable, so the image is built once (`scripts/build-runtime.ps1`) and shipped with
//! the installer. Setup then works fully offline.
//!
//! An image is a WSL-importable archive plus an optional sidecar manifest
//! (`<archive>.json`) recording its size and SHA-256. Checks happen cheapest-first so a
//! wrong or damaged file is rejected in milliseconds, not after a slow multi-gigabyte
//! import that could only fail: name and header first, then the manifest, and the full
//! checksum last.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

/// Highest image format this build of SageDock understands.
pub const SUPPORTED_FORMAT: u32 = 1;

const FILE_PREFIX: &str = "sagedock-runtime";
const ARCHIVE_SUFFIXES: &[&str] = &[".tar.xz", ".tar.gz", ".tgz", ".tar"];

/// A real image is several gigabytes. Anything this small is certainly the wrong file.
const MIN_IMAGE_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeManifest {
    pub format: u32,
    pub sage_version: String,
    #[serde(default)]
    pub arch: Option<String>,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct RuntimeImage {
    pub archive: PathBuf,
    pub manifest: Option<RuntimeManifest>,
}

impl RuntimeImage {
    pub fn file_name(&self) -> String {
        self.archive
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// This PC's architecture, spelled the way image file names and manifests spell it.
pub fn machine_arch() -> &'static str {
    match std::env::var("PROCESSOR_ARCHITECTURE").as_deref() {
        Ok("ARM64") => "arm64",
        _ => "x64",
    }
}

/// Looks for an image in each directory in order, returning the newest match from the
/// first directory that has one. Order expresses preference — a copy bundled with the
/// app wins over one that happens to be sitting in Downloads.
///
/// Only the top level of each directory is examined: recursively crawling a user's
/// Downloads or Desktop would be slow and is no business of a notebook app.
pub fn find_image(search_dirs: &[PathBuf]) -> Option<PathBuf> {
    search_dirs.iter().find_map(|dir| newest_candidate_in(dir))
}

fn newest_candidate_in(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|entry| is_candidate_name(&entry.file_name().to_string_lossy()))
        .max_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok())
        .map(|entry| entry.path())
}

fn is_candidate_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with(FILE_PREFIX) && has_archive_suffix(&name) && !names_other_arch(&name)
}

fn has_archive_suffix(lower_name: &str) -> bool {
    ARCHIVE_SUFFIXES
        .iter()
        .any(|suffix| lower_name.ends_with(suffix))
}

fn names_other_arch(lower_name: &str) -> bool {
    let other = if machine_arch() == "arm64" {
        "-x64."
    } else {
        "-arm64."
    };
    lower_name.contains(other)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveKind {
    Xz,
    Gzip,
    Tar,
}

/// Identifies the archive type from its leading bytes rather than trusting the extension.
fn sniff(header: &[u8]) -> Option<ArchiveKind> {
    if header.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]) {
        Some(ArchiveKind::Xz)
    } else if header.starts_with(&[0x1F, 0x8B]) {
        Some(ArchiveKind::Gzip)
    } else if header.len() >= 262 && &header[257..262] == b"ustar" {
        Some(ArchiveKind::Tar)
    } else {
        None
    }
}

fn manifest_path(archive: &Path) -> PathBuf {
    let mut name = archive.as_os_str().to_owned();
    name.push(".json");
    PathBuf::from(name)
}

/// Cheap validation: is this plausibly a SageDock image, made for this PC and this app?
/// Does not hash the file — see `verify` for that.
pub fn inspect(path: &Path) -> AppResult<RuntimeImage> {
    let metadata =
        std::fs::metadata(path).map_err(|err| unreadable_error(path, err.to_string()))?;
    let lower_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    let mut header = Vec::with_capacity(512);
    File::open(path)
        .and_then(|file| file.take(512).read_to_end(&mut header))
        .map_err(|err| unreadable_error(path, err.to_string()))?;

    check_candidate(metadata.is_file(), &lower_name, &header, metadata.len())?;

    let manifest = load_manifest(path)?;
    if let Some(manifest) = &manifest {
        check_manifest(manifest, metadata.len(), machine_arch())?;
    }

    Ok(RuntimeImage {
        archive: path.to_path_buf(),
        manifest,
    })
}

fn check_candidate(is_file: bool, lower_name: &str, header: &[u8], len: u64) -> AppResult<()> {
    if !is_file
        || !has_archive_suffix(lower_name)
        || sniff(header).is_none()
        || len < MIN_IMAGE_BYTES
    {
        return Err(wrong_file_error());
    }
    Ok(())
}

fn load_manifest(archive: &Path) -> AppResult<Option<RuntimeManifest>> {
    let path = manifest_path(archive);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(unreadable_error(&path, err.to_string())),
    };

    // Tolerate a byte-order mark: the manifest is a file a person might open and re-save in
    // Notepad before shipping it.
    serde_json::from_str(raw.trim_start_matches('\u{feff}'))
        .map(Some)
        .map_err(|err| damaged_error(format!("manifest could not be read: {err}")))
}

fn check_manifest(manifest: &RuntimeManifest, actual_len: u64, arch: &str) -> AppResult<()> {
    if manifest.format > SUPPORTED_FORMAT {
        return Err(needs_newer_app_error());
    }
    if let Some(image_arch) = &manifest.arch {
        if !image_arch.eq_ignore_ascii_case(arch) {
            return Err(wrong_arch_error(image_arch));
        }
    }
    // A size mismatch is the signature of an interrupted copy, and catches it without
    // hashing gigabytes first.
    if manifest.size != actual_len {
        return Err(damaged_error(format!(
            "expected {} bytes, found {actual_len}",
            manifest.size
        )));
    }
    if manifest.sha256.len() != 64 || !manifest.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(damaged_error("manifest checksum is malformed".into()));
    }
    Ok(())
}

/// Confirms the archive's SHA-256 matches its manifest, reporting progress as 0.0–1.0.
///
/// An image with no manifest is allowed through with a logged warning rather than blocked:
/// it is a local file the user explicitly chose, and the post-import checks still refuse
/// anything that isn't a working SageDock environment.
pub fn verify(image: &RuntimeImage, on_progress: &mut dyn FnMut(f32)) -> AppResult<()> {
    let Some(manifest) = &image.manifest else {
        return Err(damaged_error("A release checksum manifest is required. Copy the matching .json file next to the archive.".into()));
    };

    let approved: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../trusted-runtimes.json"))
            .map_err(|e| damaged_error(e.to_string()))?;
    if !approved.iter().any(|entry| {
        entry["sha256"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case(&manifest.sha256))
            && entry["size"].as_u64() == Some(manifest.size)
            && entry["arch"].as_str() == manifest.arch.as_deref()
    }) {
        return Err(AppError::new("runtime", "UNTRUSTED_RUNTIME", "This SageMath package isn't approved for this release",
            "Use the SageMath package supplied with this SageDock release. No code from this package has been run and your notebooks are safe."));
    }

    verify_digest(image, manifest, on_progress)
}

fn verify_digest(
    image: &RuntimeImage,
    manifest: &RuntimeManifest,
    on_progress: &mut dyn FnMut(f32),
) -> AppResult<()> {
    let mut file = File::open(&image.archive)
        .map_err(|err| unreadable_error(&image.archive, err.to_string()))?;
    let total = manifest.size.max(1);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut done: u64 = 0;
    let mut last_percent = u64::MAX;

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| unreadable_error(&image.archive, err.to_string()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        done += read as u64;

        // Whole percentages only: hashing gigabytes would otherwise push tens of thousands
        // of progress events across the IPC bridge.
        let percent = done * 100 / total;
        if percent != last_percent {
            last_percent = percent;
            on_progress(done as f32 / total as f32);
        }
    }

    let actual = hex_encode(&hasher.finalize());
    if !actual.eq_ignore_ascii_case(&manifest.sha256) {
        return Err(damaged_error(format!(
            "sha256 mismatch: expected {}, got {actual}",
            manifest.sha256
        )));
    }

    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

// --- errors --------------------------------------------------------------------------

fn wrong_file_error() -> AppError {
    AppError::new(
        "runtime",
        "SAGE_PACKAGE_WRONG_FILE",
        "That file isn't the SageMath package",
        "SageDock needs its SageMath package file, which is named like sagedock-runtime-sage10.9-x64.tar.xz. The file you chose is a different file.",
    )
}

pub fn needs_newer_app_error() -> AppError {
    AppError::new(
        "runtime",
        "SAGE_PACKAGE_TOO_NEW",
        "This SageMath package needs a newer SageDock",
        "The SageMath package was made for a newer version of SageDock. Update SageDock, then try again.",
    )
}

fn wrong_arch_error(image_arch: &str) -> AppError {
    AppError::new(
        "runtime",
        "SAGE_PACKAGE_WRONG_ARCH",
        "This SageMath package is for a different kind of PC",
        "The SageMath package you chose was made for a different type of processor than this computer has. You'll need the version made for this PC.",
    )
    .with_technical_details(format!("package: {image_arch}, this PC: {}", machine_arch()))
}

fn damaged_error(details: String) -> AppError {
    AppError::new(
        "runtime",
        "SAGE_PACKAGE_DAMAGED",
        "The SageMath package is incomplete or damaged",
        "The file looks like it was only partly copied or downloaded, or it has been damaged. Copy it again from the original and try again.",
    )
    .with_technical_details(details)
}

fn unreadable_error(path: &Path, details: String) -> AppError {
    AppError::new(
        "runtime",
        "SAGE_PACKAGE_UNREADABLE",
        "SageDock couldn't open the SageMath package",
        "SageDock wasn't able to read that file. If it's on a USB drive or a network location, copy it onto this PC first.",
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
        let dir = std::env::temp_dir().join(format!("sagedock-image-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn other_arch() -> &'static str {
        if machine_arch() == "arm64" {
            "x64"
        } else {
            "arm64"
        }
    }

    fn manifest(size: u64, arch: Option<&str>) -> RuntimeManifest {
        RuntimeManifest {
            format: 1,
            sage_version: "10.9".into(),
            arch: arch.map(String::from),
            size,
            sha256: "a".repeat(64),
        }
    }

    #[test]
    fn sniffs_archive_types_from_their_bytes() {
        assert_eq!(
            sniff(&[0xFD, b'7', b'z', b'X', b'Z', 0x00, 1, 2]),
            Some(ArchiveKind::Xz)
        );
        assert_eq!(sniff(&[0x1F, 0x8B, 8, 0]), Some(ArchiveKind::Gzip));

        let mut tar = vec![0u8; 512];
        tar[257..262].copy_from_slice(b"ustar");
        assert_eq!(sniff(&tar), Some(ArchiveKind::Tar));
    }

    #[test]
    fn rejects_non_archives_by_content() {
        assert_eq!(sniff(b"PK\x03\x04 a zip file"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn candidate_names_match_the_expected_pattern() {
        let arch = machine_arch();
        assert!(is_candidate_name(&format!(
            "sagedock-runtime-sage10.9-{arch}.tar.xz"
        )));
        assert!(is_candidate_name("SageDock-Runtime-sage10.9.TAR.GZ"));
        assert!(!is_candidate_name("sagedock-runtime-sage10.9.tar.xz.json"));
        assert!(!is_candidate_name("ubuntu-base-24.04.5-base-amd64.tar.gz"));
    }

    /// An image built for the other processor type would import and then fail to run.
    #[test]
    fn candidates_for_the_other_architecture_are_skipped() {
        assert!(!is_candidate_name(&format!(
            "sagedock-runtime-sage10.9-{}.tar.xz",
            other_arch()
        )));
    }

    #[test]
    fn find_image_prefers_earlier_directories() {
        let bundled = temp_dir();
        let downloads = temp_dir();
        let arch = machine_arch();
        std::fs::write(
            bundled.join(format!("sagedock-runtime-sage10.9-{arch}.tar.xz")),
            b"x",
        )
        .unwrap();
        std::fs::write(
            downloads.join(format!("sagedock-runtime-sage10.9-{arch}.tar.xz")),
            b"x",
        )
        .unwrap();

        let found = find_image(&[bundled.clone(), downloads]).unwrap();
        assert!(found.starts_with(&bundled), "got {}", found.display());
    }

    #[test]
    fn find_image_ignores_unrelated_files_and_missing_directories() {
        let dir = temp_dir();
        std::fs::write(dir.join("homework.pdf"), b"x").unwrap();
        std::fs::write(dir.join("sagedock-runtime-notes.txt"), b"x").unwrap();

        assert_eq!(
            find_image(&[PathBuf::from(""), dir.join("does-not-exist"), dir]),
            None
        );
    }

    #[test]
    fn check_candidate_rejects_small_or_mislabelled_files() {
        let gz = [0x1F, 0x8B, 8, 0];
        assert!(check_candidate(true, "sagedock-runtime.tar.gz", &gz, MIN_IMAGE_BYTES).is_ok());
        assert!(check_candidate(true, "sagedock-runtime.tar.gz", &gz, 1024).is_err());
        assert!(check_candidate(
            true,
            "sagedock-runtime.tar.gz",
            b"not gzip",
            MIN_IMAGE_BYTES
        )
        .is_err());
        assert!(check_candidate(true, "sagedock-runtime.zip", &gz, MIN_IMAGE_BYTES).is_err());
        assert!(check_candidate(false, "sagedock-runtime.tar.gz", &gz, MIN_IMAGE_BYTES).is_err());
    }

    #[test]
    fn manifest_checks() {
        let arch = machine_arch();
        assert!(check_manifest(&manifest(100, Some(arch)), 100, arch).is_ok());
        assert!(check_manifest(&manifest(100, None), 100, arch).is_ok());

        let too_new = RuntimeManifest {
            format: SUPPORTED_FORMAT + 1,
            ..manifest(100, None)
        };
        assert_eq!(
            check_manifest(&too_new, 100, arch).unwrap_err().code,
            "SAGE_PACKAGE_TOO_NEW"
        );

        assert_eq!(
            check_manifest(&manifest(100, Some(other_arch())), 100, arch)
                .unwrap_err()
                .code,
            "SAGE_PACKAGE_WRONG_ARCH"
        );
        assert_eq!(
            check_manifest(&manifest(100, None), 99, arch)
                .unwrap_err()
                .code,
            "SAGE_PACKAGE_DAMAGED"
        );

        let bad_sha = RuntimeManifest {
            sha256: "xyz".into(),
            ..manifest(100, None)
        };
        assert_eq!(
            check_manifest(&bad_sha, 100, arch).unwrap_err().code,
            "SAGE_PACKAGE_DAMAGED"
        );
    }

    #[test]
    fn manifest_sits_next_to_the_archive() {
        assert_eq!(
            manifest_path(Path::new(r"C:\x\sagedock-runtime.tar.xz")),
            PathBuf::from(r"C:\x\sagedock-runtime.tar.xz.json")
        );
    }

    #[test]
    fn loads_a_manifest_with_a_byte_order_mark() {
        let dir = temp_dir();
        let archive = dir.join("sagedock-runtime.tar.xz");
        let json = format!(
            "\u{feff}{{\"format\":1,\"sage_version\":\"10.9\",\"size\":5,\"sha256\":\"{}\"}}",
            "a".repeat(64)
        );
        std::fs::write(manifest_path(&archive), json).unwrap();

        let loaded = load_manifest(&archive)
            .unwrap()
            .expect("manifest should load");
        assert_eq!(loaded.sage_version, "10.9");
    }

    #[test]
    fn missing_manifest_is_not_an_error() {
        let dir = temp_dir();
        assert!(load_manifest(&dir.join("sagedock-runtime.tar.xz"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn verify_accepts_a_matching_checksum_and_rejects_a_wrong_one() {
        let dir = temp_dir();
        let archive = dir.join("sample.tar.gz");
        std::fs::write(&archive, b"hello").unwrap();
        // Known SHA-256 of "hello".
        let good = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

        let mut image = RuntimeImage {
            archive,
            manifest: Some(RuntimeManifest {
                sha256: good.into(),
                ..manifest(5, None)
            }),
        };
        let mut reports = 0;
        assert!(
            verify_digest(&image, image.manifest.as_ref().unwrap(), &mut |_| {
                reports += 1
            })
            .is_ok()
        );
        assert!(reports > 0, "progress should be reported");

        image.manifest.as_mut().unwrap().sha256 = "0".repeat(64);
        assert_eq!(
            verify_digest(&image, image.manifest.as_ref().unwrap(), &mut |_| {})
                .unwrap_err()
                .code,
            "SAGE_PACKAGE_DAMAGED"
        );
    }

    #[test]
    fn verify_without_a_manifest_is_refused() {
        let image = RuntimeImage {
            archive: PathBuf::from("unused"),
            manifest: None,
        };
        assert!(verify(&image, &mut |_| {}).is_err());
    }
}
