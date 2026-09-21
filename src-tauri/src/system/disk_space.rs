//! General free-space warning for the Windows system drive, queried with the Win32 API.
//! Setup and backup operations separately check the space required at their actual
//! destinations; this diagnostic is an early warning, not an installation guarantee.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

use crate::error::ErrorSeverity;
use crate::system::types::CheckItem;

const BYTES_PER_GB: u64 = 1024 * 1024 * 1024;
const LOW_SPACE_GB: u64 = 5;
const TIGHT_SPACE_GB: u64 = 15;

fn gather() -> Option<u64> {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    available(std::path::Path::new(&format!("{drive}\\")))
}

pub fn available(directory: &std::path::Path) -> Option<u64> {
    let mut existing = directory;
    while !existing.exists() {
        existing = existing.parent()?;
    }
    let path: Vec<u16> = OsStr::new(existing.as_os_str())
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut free_bytes_available: u64 = 0;
    // SAFETY: `path` is a valid null-terminated wide string; the three out-pointers are
    // valid `u64` locals we own for the duration of the call. We only read the first
    // (free space available to the caller), so the other two use throwaway locals.
    let ok = unsafe {
        let mut total_bytes: u64 = 0;
        let mut total_free_bytes: u64 = 0;
        GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut free_bytes_available,
            &mut total_bytes,
            &mut total_free_bytes,
        )
    };

    if ok == 0 {
        None
    } else {
        Some(free_bytes_available)
    }
}

fn interpret(free_bytes: Option<u64>) -> CheckItem {
    let Some(free_bytes) = free_bytes else {
        return CheckItem {
            id: "disk_space",
            label: "Free disk space".into(),
            severity: ErrorSeverity::Warning,
            summary: "SageDock couldn't check how much free space is available.".into(),
            detail: None,
        };
    };

    let free_gb = free_bytes / BYTES_PER_GB;
    let detail = Some(format!("{free_gb} GB free"));

    if free_gb < LOW_SPACE_GB {
        CheckItem {
            id: "disk_space",
            label: "Free disk space".into(),
            severity: ErrorSeverity::Error,
            summary: format!(
                "SageDock needs more free space. Only {free_gb} GB is available — free up some space before setting up SageMath."
            ),
            detail,
        }
    } else if free_gb < TIGHT_SPACE_GB {
        CheckItem {
            id: "disk_space",
            label: "Free disk space".into(),
            severity: ErrorSeverity::Warning,
            summary: format!(
                "You have {free_gb} GB free. That's enough for now, but installing SageMath and Jupyter will need more room."
            ),
            detail,
        }
    } else {
        CheckItem {
            id: "disk_space",
            label: "Free disk space".into(),
            severity: ErrorSeverity::Info,
            summary: format!("You have enough storage available ({free_gb} GB free)."),
            detail,
        }
    }
}

pub fn check() -> CheckItem {
    interpret(gather())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plenty_of_space_is_healthy() {
        assert_eq!(
            interpret(Some(200 * BYTES_PER_GB)).severity,
            ErrorSeverity::Info
        );
    }

    #[test]
    fn tight_space_is_a_warning() {
        assert_eq!(
            interpret(Some(10 * BYTES_PER_GB)).severity,
            ErrorSeverity::Warning
        );
    }

    #[test]
    fn very_low_space_is_an_error() {
        assert_eq!(
            interpret(Some(2 * BYTES_PER_GB)).severity,
            ErrorSeverity::Error
        );
    }

    #[test]
    fn undetectable_space_is_a_soft_warning() {
        assert_eq!(interpret(None).severity, ErrorSeverity::Warning);
    }
}
