//! Which web browsers are installed on this PC, so first-run setup can offer a real choice
//! instead of only "your default browser".
//!
//! Windows records every installed browser under `SOFTWARE\Clients\StartMenuInternet`,
//! the same list the Settings app's "Default apps" page reads. Both hives are consulted
//! because a browser installed for one user only appears under `HKEY_CURRENT_USER`, and a
//! machine-wide one under `HKEY_LOCAL_MACHINE`.
//!
//! Follows the gather/interpret split used throughout `system/`: [`installed`] performs the
//! registry read and is deliberately not unit-tested, because its result depends on the
//! machine, while [`executable_from_command`] is pure and is tested exhaustively.
//!
//! **The frontend never names an executable.** It sends back one of the identifiers this
//! module handed out, and [`open`] re-resolves that identifier against the registry to find
//! the program to run. That keeps the rule the rest of the codebase follows, the webview
//! cannot name a file for SageDock to execute, intact for this feature too.

use std::path::PathBuf;

use serde::Serialize;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::RegKey;

use crate::error::{AppError, AppResult};

const CLIENTS_KEY: &str = r"SOFTWARE\Clients\StartMenuInternet";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Browser {
    /// The registry subkey, used as the stable identifier stored in settings. Usually a
    /// readable name such as `Google Chrome`, but some installers use a suffixed key like
    /// `Firefox-308046B0AF4A39CB`, which is why it is kept apart from the display name.
    pub id: String,
    /// What to show the student, the key's default value, falling back to the identifier.
    pub name: String,
}

/// Every browser Windows knows about whose executable actually exists on disk.
///
/// A registry entry left behind by an uninstalled browser is skipped rather than offered:
/// a list that includes a browser which cannot start is worse than a shorter honest one.
pub fn installed() -> Vec<Browser> {
    let mut found: Vec<Browser> = Vec::new();

    // Machine-wide first: where a browser is registered in both hives, the identifiers are
    // the same and the entries describe the same program, so the first one seen wins.
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let Ok(clients) = RegKey::predef(hive).open_subkey(CLIENTS_KEY) else {
            continue;
        };
        for id in clients.enum_keys().flatten() {
            if found.iter().any(|b| b.id.eq_ignore_ascii_case(&id)) {
                continue;
            }
            let Ok(entry) = clients.open_subkey(&id) else {
                continue;
            };
            let Some(exe) = executable_of(&entry) else {
                continue;
            };
            if !exe.is_file() {
                continue;
            }
            let name = entry
                .get_value::<String, _>("")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| id.clone());
            found.push(Browser { id, name });
        }
    }

    found.sort_by_key(|b| b.name.to_lowercase());
    found
}

/// Opens a URL in one specific installed browser.
///
/// The identifier is resolved back to an executable here rather than being taken on trust,
/// and the URL is passed as a single argument to the program, never through a shell, and
/// never concatenated into a command string, the same rule `wsl.rs` follows.
pub fn open(id: &str, url: &str) -> AppResult<()> {
    let exe = locate(id).ok_or_else(|| missing_browser_error(id))?;

    std::process::Command::new(&exe)
        .arg(url)
        .spawn()
        .map_err(|err| {
            AppError::new(
                "launcher",
                "BROWSER_LAUNCH_FAILED",
                "SageDock couldn't open your chosen browser",
                "SageDock wasn't able to start the browser you picked. You can choose a different one in Settings, or switch back to opening notebooks in SageDock's own window.",
            )
            .with_technical_details(format!("{}: {err}", exe.display()))
        })?;

    Ok(())
}

/// Finds the executable registered for one browser identifier, in either hive.
fn locate(id: &str) -> Option<PathBuf> {
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let entry = RegKey::predef(hive)
            .open_subkey(CLIENTS_KEY)
            .and_then(|clients| clients.open_subkey(id));
        if let Ok(entry) = entry {
            if let Some(exe) = executable_of(&entry) {
                if exe.is_file() {
                    return Some(exe);
                }
            }
        }
    }
    None
}

/// Reads `<browser>\shell\open\command` and extracts the program from it.
fn executable_of(entry: &RegKey) -> Option<PathBuf> {
    let command: String = entry
        .open_subkey(r"shell\open\command")
        .ok()?
        .get_value("")
        .ok()?;
    executable_from_command(&command).map(PathBuf::from)
}

/// Pulls the program out of a registered shell command.
///
/// The value is a command line, not a path: it is usually a quoted executable, sometimes
/// followed by switches, and occasionally unquoted. Cutting an unquoted value at the first
/// space would truncate `C:\Program Files\...` at "Program", so it is cut after the `.exe`
/// instead. Returns `None` for anything that doesn't name a program, rather than guessing.
fn executable_from_command(command: &str) -> Option<&str> {
    let trimmed = command.trim();

    if let Some(rest) = trimmed.strip_prefix('"') {
        let end = rest.find('"')?;
        let path = &rest[..end];
        return (!path.is_empty()).then_some(path);
    }

    let end = trimmed.to_ascii_lowercase().find(".exe").map(|i| i + 4)?;
    Some(&trimmed[..end])
}

fn missing_browser_error(id: &str) -> AppError {
    AppError::new(
        "launcher",
        "BROWSER_NOT_FOUND",
        "SageDock couldn't find your chosen browser",
        "The browser you picked isn't installed on this PC any more. Choose a different one in Settings, or switch back to opening notebooks in SageDock's own window.",
    )
    .with_technical_details(format!("no StartMenuInternet entry resolved for {id}"))
}

#[cfg(test)]
mod tests {
    use super::executable_from_command;

    #[test]
    fn a_quoted_program_with_spaces_in_its_path_is_read_whole() {
        assert_eq!(
            executable_from_command(r#""C:\Program Files\Google\Chrome\Application\chrome.exe""#),
            Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe")
        );
    }

    /// The common real-world shape: a quoted program followed by switches, which must not
    /// end up treated as part of the path.
    #[test]
    fn switches_after_a_quoted_program_are_dropped() {
        assert_eq!(
            executable_from_command(
                r#""C:\Program Files\Mozilla Firefox\firefox.exe" -osint -url"#
            ),
            Some(r"C:\Program Files\Mozilla Firefox\firefox.exe")
        );
    }

    /// The case that makes cutting at the first space wrong: an unquoted path whose
    /// directory contains a space would otherwise be truncated to `C:\Program`.
    #[test]
    fn an_unquoted_program_is_cut_after_the_extension_not_at_the_first_space() {
        assert_eq!(
            executable_from_command(r"C:\Program Files\Internet Explorer\iexplore.exe"),
            Some(r"C:\Program Files\Internet Explorer\iexplore.exe")
        );
    }

    #[test]
    fn switches_after_an_unquoted_program_are_dropped() {
        assert_eq!(
            executable_from_command(r"C:\Windows\system32\browser.exe --new-window"),
            Some(r"C:\Windows\system32\browser.exe")
        );
    }

    #[test]
    fn the_extension_is_matched_regardless_of_casing() {
        assert_eq!(
            executable_from_command(r"C:\Apps\Browser.EXE /launch"),
            Some(r"C:\Apps\Browser.EXE")
        );
    }

    /// A malformed or empty entry must be skipped rather than turned into a path that
    /// SageDock would then try to execute.
    #[test]
    fn entries_that_do_not_name_a_program_are_refused() {
        assert_eq!(executable_from_command(""), None);
        assert_eq!(executable_from_command("   "), None);
        assert_eq!(executable_from_command(r#""""#), None);
        assert_eq!(executable_from_command("notaprogram"), None);
    }
}
