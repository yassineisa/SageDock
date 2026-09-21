//! Application configuration storage.
//!
//! Settings are persisted as a single JSON file under the app's config directory
//! (Tauri's per-OS app config path, not a hardcoded path). A missing or corrupt
//! config file is never treated as fatal — SageDock falls back to defaults and logs
//! a warning, since losing a settings file should never block the user from getting
//! into the app. Writes are atomic (write to a temp file, then rename) so a crash or
//! power loss mid-save can't leave a half-written config behind.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult, ErrorSeverity};

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub theme: ThemePreference,
    #[serde(default)]
    pub open_in_browser: bool,
    /// Whether the first-run introduction has been finished or skipped.
    ///
    /// Every new field here is `#[serde(default)]`, so a settings file written by an older
    /// SageDock loads without this key and reads as `false` — which means an existing
    /// installation shows the introduction once after updating. That is deliberate: the
    /// alternative is defaulting to `true`, which would hide a first-run explanation from
    /// the one group of users who have never seen it either.
    #[serde(default)]
    pub onboarding_complete: bool,
    /// Which installed browser to open notebooks in, as the registry client name reported
    /// by `browsers::installed` (for example `Google Chrome`).
    ///
    /// `None` means "whatever Windows has set as the default", which is what SageDock did
    /// before this setting existed and remains the behaviour for anyone who never picks
    /// one. Only meaningful when `open_in_browser` is set; the two are kept separate so
    /// switching back to the SageDock window doesn't discard the chosen browser.
    #[serde(default)]
    pub preferred_browser: Option<String>,
}

fn default_schema_version() -> u32 {
    CONFIG_SCHEMA_VERSION
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            theme: ThemePreference::default(),
            open_in_browser: false,
            onboarding_complete: false,
            preferred_browser: None,
        }
    }
}

pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            path: config_dir.join("config.json"),
        }
    }

    /// Loads settings from disk. Never fails: a missing file returns defaults
    /// silently, and a corrupt file returns defaults after logging a warning.
    pub fn load(&self) -> AppConfig {
        let raw = match fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return AppConfig::default();
            }
            Err(err) => {
                tracing::warn!(target: "config", error = %err, path = %self.path.display(), "could not read settings file, using defaults");
                return AppConfig::default();
            }
        };

        match serde_json::from_str(&raw) {
            Ok(config) => config,
            Err(err) => {
                tracing::warn!(target: "config", error = %err, path = %self.path.display(), "settings file was unreadable, using defaults");
                AppConfig::default()
            }
        }
    }

    /// Persists settings to disk atomically. Failures are real, user-visible errors
    /// since a save the user asked for silently not happening would be surprising.
    pub fn save(&self, config: &AppConfig) -> AppResult<()> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|err| config_write_error(err.to_string()))?;

        let json = serde_json::to_string_pretty(config)
            .map_err(|err| config_write_error(err.to_string()))?;

        crate::storage::atomic_write(&self.path, json.as_bytes())
            .map_err(|err| config_write_error(err.to_string()))?;

        tracing::info!(target: "config", "settings saved");
        Ok(())
    }
}

fn config_write_error(technical_details: String) -> AppError {
    tracing::error!(target: "config", error = %technical_details, "failed to save settings");
    AppError::new(
        "config",
        "CONFIG_WRITE_FAILED",
        "Your changes could not be saved",
        "SageDock could not save your settings. Check that there is free disk space and that SageDock has permission to write to its settings folder, then try again.",
    )
    .with_severity(ErrorSeverity::Warning)
    .with_technical_details(technical_details)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Unique per-test scratch directory so parallel `cargo test` runs don't clobber
    /// each other's config.json.
    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("sagedock-config-test-{}-{}", std::process::id(), n));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn load_returns_defaults_when_file_is_missing() {
        let store = ConfigStore::new(&temp_dir());
        let config = store.load();
        assert_eq!(config.theme, ThemePreference::System);
        assert_eq!(config.schema_version, CONFIG_SCHEMA_VERSION);
    }

    #[test]
    fn save_then_load_round_trips() {
        let store = ConfigStore::new(&temp_dir());
        let config = AppConfig {
            theme: ThemePreference::Dark,
            ..AppConfig::default()
        };

        store.save(&config).expect("save should succeed");
        let loaded = store.load();

        assert_eq!(loaded.theme, ThemePreference::Dark);
    }

    #[test]
    fn load_falls_back_to_defaults_on_corrupt_file() {
        let dir = temp_dir();
        let store = ConfigStore::new(&dir);
        fs::write(dir.join("config.json"), "{ not valid json").unwrap();

        let loaded = store.load();

        assert_eq!(loaded.theme, ThemePreference::System);
    }

    /// A settings file written by a SageDock that predates the introduction and the browser
    /// choice must still load, keep what it did say, and must not claim the student has
    /// already been shown an introduction that did not exist when it was written.
    #[test]
    fn settings_from_an_earlier_version_load_with_the_new_fields_defaulted() {
        let dir = temp_dir();
        let store = ConfigStore::new(&dir);
        fs::write(
            dir.join("config.json"),
            r#"{"schema_version":1,"theme":"dark","open_in_browser":true}"#,
        )
        .unwrap();

        let loaded = store.load();

        assert_eq!(loaded.theme, ThemePreference::Dark);
        assert!(loaded.open_in_browser);
        assert!(!loaded.onboarding_complete);
        assert_eq!(loaded.preferred_browser, None);
    }

    #[test]
    fn a_finished_introduction_and_chosen_browser_survive_a_round_trip() {
        let store = ConfigStore::new(&temp_dir());
        store
            .save(&AppConfig {
                onboarding_complete: true,
                preferred_browser: Some("Google Chrome".into()),
                ..AppConfig::default()
            })
            .expect("save should succeed");

        let loaded = store.load();

        assert!(loaded.onboarding_complete);
        assert_eq!(loaded.preferred_browser.as_deref(), Some("Google Chrome"));
    }

    #[test]
    fn save_does_not_leave_a_temp_file_behind() {
        let dir = temp_dir();
        let store = ConfigStore::new(&dir);
        store
            .save(&AppConfig::default())
            .expect("save should succeed");

        assert!(dir.join("config.json").exists());
        assert!(!dir.join("config.json.tmp").exists());
    }
}
