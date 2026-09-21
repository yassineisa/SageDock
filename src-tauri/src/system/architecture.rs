//! Processor architecture check. Mostly informational: this binary is compiled for
//! 64-bit Windows, so it could not have launched at all on an unsupported (32-bit)
//! architecture. It still surfaces which 64-bit architecture (AMD64 vs ARM64) for
//! diagnostics/support purposes, since that matters for which WSL/SageMath builds
//! provisioning can install.

use crate::error::ErrorSeverity;
use crate::system::types::CheckItem;

fn gather() -> Option<String> {
    std::env::var("PROCESSOR_ARCHITECTURE").ok()
}

fn interpret(arch: Option<String>) -> CheckItem {
    match arch.as_deref() {
        Some("AMD64") => CheckItem {
            id: "architecture",
            label: "Processor architecture".into(),
            severity: ErrorSeverity::Info,
            summary: "Your computer's processor is supported (64-bit).".into(),
            detail: Some("AMD64".into()),
        },
        Some("ARM64") => CheckItem {
            id: "architecture",
            label: "Processor architecture".into(),
            severity: ErrorSeverity::Info,
            summary: "Your computer's processor is supported (ARM64).".into(),
            detail: Some("ARM64".into()),
        },
        Some(other) => CheckItem {
            id: "architecture",
            label: "Processor architecture".into(),
            severity: ErrorSeverity::Warning,
            summary: "SageDock couldn't confirm your processor type.".into(),
            detail: Some(other.to_string()),
        },
        None => CheckItem {
            id: "architecture",
            label: "Processor architecture".into(),
            severity: ErrorSeverity::Warning,
            summary: "SageDock couldn't confirm your processor type.".into(),
            detail: None,
        },
    }
}

pub fn check() -> CheckItem {
    interpret(gather())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amd64_is_healthy() {
        assert_eq!(
            interpret(Some("AMD64".into())).severity,
            ErrorSeverity::Info
        );
    }

    #[test]
    fn arm64_is_healthy() {
        assert_eq!(
            interpret(Some("ARM64".into())).severity,
            ErrorSeverity::Info
        );
    }

    #[test]
    fn unknown_value_is_a_soft_warning() {
        assert_eq!(
            interpret(Some("x86".into())).severity,
            ErrorSeverity::Warning
        );
    }

    #[test]
    fn missing_env_var_is_a_soft_warning() {
        assert_eq!(interpret(None).severity, ErrorSeverity::Warning);
    }
}
