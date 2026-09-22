//! Creating new notebook files.
//!
//! Notebooks are written on the Windows side, directly into the user's workspace, in
//! standard `.ipynb` format. Nothing here is SageDock-specific: the files remain ordinary
//! Jupyter notebooks that open in any Jupyter, which is the point, the product spec rules
//! out a proprietary notebook format.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotebookKind {
    Sage,
    Python,
}

impl NotebookKind {
    /// Jupyter kernelspec name. `sagemath` is the kernel the Debian/Ubuntu SageMath
    /// packaging registers; `python3` is the stock IPython kernel.
    fn kernel_name(self) -> &'static str {
        match self {
            NotebookKind::Sage => "sagemath",
            NotebookKind::Python => "python3",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            NotebookKind::Sage => "SageMath",
            NotebookKind::Python => "Python 3",
        }
    }

    fn language(self) -> &'static str {
        match self {
            NotebookKind::Sage => "sage",
            NotebookKind::Python => "python",
        }
    }

    /// Default file name stem, in the user's language rather than the kernel's.
    fn default_stem(self) -> &'static str {
        match self {
            NotebookKind::Sage => "Sage Notebook",
            NotebookKind::Python => "Python Notebook",
        }
    }
}

/// Builds an empty notebook document for the given kernel.
fn notebook_json(kind: NotebookKind) -> serde_json::Value {
    serde_json::json!({
        "cells": [{
            "cell_type": "code",
            "execution_count": null,
            "metadata": {},
            "outputs": [],
            "source": []
        }],
        "metadata": {
            "kernelspec": {
                "display_name": kind.display_name(),
                "language": kind.language(),
                "name": kind.kernel_name()
            }
        },
        "nbformat": 4,
        "nbformat_minor": 5
    })
}

/// How many names to try before giving up rather than inventing an unchecked one.
const MAX_NAME_ATTEMPTS: u32 = 10_000;

/// Creates a new notebook directly in the workspace folder and returns its path
/// relative to the workspace root (which is what Jupyter URLs are built from).
///
/// Never overwrites an existing notebook, and that guarantee comes from the operating
/// system rather than from a prior existence check. Asking "does this name exist?" and then
/// writing is a race: two notebook requests moments apart can both observe the same free
/// name, and the second write silently truncates the first. `create_new` makes the create
/// atomic, exactly one caller can win a given name, and the loser simply tries the next.
pub fn create_notebook(workspace_dir: &Path, kind: NotebookKind) -> AppResult<String> {
    let folder = workspace_dir;
    std::fs::create_dir_all(folder).map_err(|err| create_error(err.to_string()))?;

    let json = serde_json::to_string_pretty(&notebook_json(kind))
        .map_err(|err| create_error(err.to_string()))?;

    for attempt in 0..MAX_NAME_ATTEMPTS {
        let path = candidate_path(folder, kind.default_stem(), attempt);

        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(json.as_bytes())
                    .map_err(|err| create_error(err.to_string()))?;

                let file_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                tracing::info!(target: "notebook", kind = ?kind, file = %file_name, "created notebook");
                return Ok(file_name);
            }
            // Someone else holds this name, including a notebook the user made earlier.
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(create_error(err.to_string())),
        }
    }

    // Reached only if thousands of names are taken. Failing is the safe outcome; the old
    // fallback invented a name and wrote it without checking, which could truncate a file.
    Err(AppError::new(
        "notebook",
        "NOTEBOOK_NAME_EXHAUSTED",
        "SageDock couldn't find a name for your notebook",
        "Your notebooks folder already contains a great many notebooks with similar names. Renaming or moving some of them will let SageDock add a new one.",
    ))
}

/// "Sage Notebook.ipynb", then "Sage Notebook 2.ipynb", "Sage Notebook 3.ipynb", …
fn candidate_path(folder: &Path, stem: &str, attempt: u32) -> PathBuf {
    if attempt == 0 {
        folder.join(format!("{stem}.ipynb"))
    } else {
        folder.join(format!("{stem} {}.ipynb", attempt + 1))
    }
}

fn create_error(details: String) -> AppError {
    AppError::new(
        "notebook",
        "NOTEBOOK_CREATE_FAILED",
        "SageDock couldn't create your notebook",
        "SageDock wasn't able to save a new notebook to your SageDock folder. Check that you have free disk space and that the folder isn't read-only.",
    )
    .with_technical_details(details)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_workspace() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("sagedock-nb-test-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The data-safety guarantee under concurrency: every caller must get its own file, and
    /// no write may land on top of another. The previous check-then-write could give two
    /// callers the same name, and the second silently destroyed the first.
    #[test]
    fn concurrent_creation_never_collides_or_truncates() {
        let ws = temp_workspace();
        let threads = 8;

        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let ws = ws.clone();
                std::thread::spawn(move || create_notebook(&ws, NotebookKind::Sage).unwrap())
            })
            .collect();

        let paths: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let unique: std::collections::HashSet<&String> = paths.iter().collect();
        assert_eq!(
            unique.len(),
            threads,
            "two notebooks were given the same name: {paths:?}"
        );

        // Every file must be complete, not a half-written or truncated leftover.
        for path in &paths {
            let raw = std::fs::read_to_string(ws.join(path)).unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&raw)
                .unwrap_or_else(|err| panic!("{path} is not valid JSON: {err}"));
            assert_eq!(parsed["metadata"]["kernelspec"]["name"], "sagemath");
        }
    }

    /// A pre-existing file must survive even when it occupies a *numbered* name rather than
    /// the default one.
    #[test]
    fn preserves_existing_files_at_numbered_names() {
        let ws = temp_workspace();
        let folder = &ws;
        std::fs::create_dir_all(folder).unwrap();
        std::fs::write(folder.join("Sage Notebook.ipynb"), b"first").unwrap();
        std::fs::write(folder.join("Sage Notebook 2.ipynb"), b"second").unwrap();

        let created = create_notebook(&ws, NotebookKind::Sage).unwrap();

        assert_eq!(created, "Sage Notebook 3.ipynb");
        assert_eq!(
            std::fs::read(folder.join("Sage Notebook.ipynb")).unwrap(),
            b"first"
        );
        assert_eq!(
            std::fs::read(folder.join("Sage Notebook 2.ipynb")).unwrap(),
            b"second"
        );
    }

    #[test]
    fn candidate_names_are_sequential() {
        let folder = Path::new("N");
        assert_eq!(
            candidate_path(folder, "Sage Notebook", 0),
            folder.join("Sage Notebook.ipynb")
        );
        assert_eq!(
            candidate_path(folder, "Sage Notebook", 1),
            folder.join("Sage Notebook 2.ipynb")
        );
        assert_eq!(
            candidate_path(folder, "Sage Notebook", 2),
            folder.join("Sage Notebook 3.ipynb")
        );
    }

    #[test]
    fn sage_notebook_declares_the_sage_kernel() {
        let json = notebook_json(NotebookKind::Sage);
        assert_eq!(json["metadata"]["kernelspec"]["name"], "sagemath");
        assert_eq!(json["nbformat"], 4);
    }

    #[test]
    fn python_notebook_declares_the_python_kernel() {
        let json = notebook_json(NotebookKind::Python);
        assert_eq!(json["metadata"]["kernelspec"]["name"], "python3");
    }

    #[test]
    fn creates_a_notebook_directly_in_the_workspace() {
        let ws = temp_workspace();
        let rel = create_notebook(&ws, NotebookKind::Sage).unwrap();

        assert_eq!(rel, "Sage Notebook.ipynb");
        assert!(ws.join("Sage Notebook.ipynb").is_file());
        assert!(!ws.join("Notebooks").exists());
    }

    /// The data-safety rule that matters most here.
    #[test]
    fn never_overwrites_an_existing_notebook() {
        let ws = temp_workspace();
        let first = create_notebook(&ws, NotebookKind::Sage).unwrap();
        std::fs::write(ws.join(&first), b"precious user work").unwrap();

        let second = create_notebook(&ws, NotebookKind::Sage).unwrap();

        assert_ne!(first, second);
        assert_eq!(
            std::fs::read(ws.join(&first)).unwrap(),
            b"precious user work"
        );
    }

    #[test]
    fn created_notebook_is_valid_json() {
        let ws = temp_workspace();
        let rel = create_notebook(&ws, NotebookKind::Python).unwrap();
        let raw = std::fs::read_to_string(ws.join(rel)).unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed["metadata"]["kernelspec"]["name"], "python3");
    }
}
