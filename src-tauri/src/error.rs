//! Structured error type returned to the frontend from every backend command.
//!
//! Per the product spec, every user-facing failure must answer: what happened,
//! is my work safe, can SageDock fix it, and what should the user do next. Commands
//! must never return raw process output or bare strings, everything funnels through
//! `AppError` so the frontend can render a consistent, friendly error screen and log
//! the technical detail separately.

use serde::Serialize;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorSeverity {
    /// Not a failure; informational only.
    Info,
    /// Degraded but the app remains usable.
    Warning,
    /// The requested operation failed.
    Error,
    /// Unrecoverable without a restart, reinstall, or administrator action.
    Fatal,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecoveryAction {
    /// Stable identifier the frontend maps to a button/handler, e.g. "repair_config".
    pub id: String,
    /// Button label, e.g. "Repair settings".
    pub label: String,
}

/// A structured, user-safe error. Every field the UI needs to render a friendly
/// error screen lives here, the frontend should never need to parse `technical_details`
/// to decide what to show.
#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    /// Stable machine-readable code, e.g. "CONFIG_READ_FAILED". Used for tests and logs,
    /// never shown directly to the user.
    pub code: String,
    /// Short friendly title, e.g. "Settings could not be loaded".
    pub title: String,
    /// One or two plain-language sentences explaining what happened.
    pub message: String,
    pub severity: ErrorSeverity,
    /// Whether the user's notebooks/projects are known to be unaffected by this failure.
    /// Defaults to `true`; operations that may affect saved user files must override it.
    pub user_files_safe: bool,
    /// Which backend area raised this (e.g. "config", "logging"), mirrors the log categories.
    pub component: String,
    /// Actions the UI can offer, ordered least to most invasive. Empty when no automated
    /// recovery exists yet.
    pub recovery_actions: Vec<RecoveryAction>,
    /// Technical detail (original error, process output) for the optional "Show details" panel.
    pub technical_details: Option<String>,
    pub timestamp: String,
}

impl AppError {
    pub fn new(
        component: impl Into<String>,
        code: impl Into<String>,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            title: title.into(),
            message: message.into(),
            severity: ErrorSeverity::Error,
            user_files_safe: true,
            component: component.into(),
            recovery_actions: Vec::new(),
            technical_details: None,
            timestamp: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_default(),
        }
    }

    pub fn with_severity(mut self, severity: ErrorSeverity) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_technical_details(mut self, details: impl Into<String>) -> Self {
        self.technical_details = Some(details.into());
        self
    }
}

pub type AppResult<T> = Result<T, AppError>;
