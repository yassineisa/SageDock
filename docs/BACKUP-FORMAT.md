# SageDock backup format

**Format version 1.** Implemented in [`src-tauri/src/backup.rs`](../src-tauri/src/backup.rs).

A SageDock backup holds a student's own work — workspaces, notebooks, datasets, and
notebook checkpoints. Its purpose is to survive things: a reinstall, a new Windows account,
a different computer.

> This is **not** the runtime backup created by Recovery → _Back up & reinstall_. That one
> snapshots the disposable Linux environment via `wsl --export`, is tied to this machine's
> WSL registration, and is not portable. The two are deliberately separate concepts.

## Portability guarantee

The generated manifest records no machine-specific source paths, and restoring does not
require any of the following. File contents are copied as-is and may themselves contain
personal paths, credentials, or other sensitive information; the ZIP is not encrypted.

- the Windows username or any absolute path
- the SageDock application configuration directory
- a WSL installation, or an existing SageDock distribution
- the SageDock version that wrote it (it is recorded, but only as information)

Restoring resolves every path relative to wherever the restoring installation keeps its
workspaces. A backup made on one PC restores on another with no rewriting.

## Container

An ordinary ZIP file, Deflate-compressed. This is deliberate: a student who needs one
notebook back can open it in File Explorer without having SageDock installed at all.

```text
sagedock-backup.json                       manifest (below)
workspaces/00-calculus/Notebooks/wk1.ipynb
workspaces/00-calculus/Notebooks/.ipynb_checkpoints/wk1-checkpoint.ipynb
workspaces/01-physics/Datasets/readings.csv
```

Each workspace becomes one top-level folder under `workspaces/`, named `NN-slug` where `NN`
is the workspace's index and `slug` is its display name reduced to `[a-z0-9-]`. The index
prefix is what stops two workspaces with the same display name from colliding. The original
relative file paths inside a workspace are preserved. Empty directories are not archived.

The manifest is written **last**, after every file entry.

## Manifest

`sagedock-backup.json`, UTF-8:

```json
{
  "format": 1,
  "app_version": "1.1.0",
  "created_utc": "2026-09-17T09:14:22Z",
  "file_count": 128,
  "total_bytes": 41234567,
  "workspaces": [
    {
      "slug": "00-calculus",
      "name": "Calculus",
      "file_count": 64,
      "total_bytes": 20480000,
      "files": [
        {
          "path": "workspaces/00-calculus/Notebooks/wk1.ipynb",
          "size": 4096,
          "sha256": "9f86d081…"
        }
      ]
    }
  ],
  "preferences": { "theme": "dark", "open_in_browser": false }
}
```

`files[].path` is the archive entry name, always forward-slashed and always relative.
`sha256` is over the file's bytes. `preferences` carries only settings meaningful on another
machine; ports, tokens, paths, and the selected SageMath package are deliberately absent.
Preference fields are retained as metadata; this release restores coursework and workspace
cards without changing the receiving installation's theme or browser preference.

## What is excluded

Included, because it is the user's work: notebooks, datasets, exports, arbitrary files and
subfolders, `.ipynb_checkpoints` (recovered coursework), and `.git` history if present.

Excluded: `.jupyter` (holds tokens), `__pycache__`, `node_modules`, `.venv` / `venv`,
`.DS_Store`, `Thumbs.db`, `*.pyc`, `*.tmp`, and anything beginning `.sagedock-` (SageDock's
own probe and partial files). Junctions, symlinks, and unsupported reparse points cause a
clear failure rather than silently omitting files or following links. Windows CLOUD reparse
tags are allowed; make those files available offline before backing up. Missing/unreadable
workspaces and trees exceeding the depth limit also fail the backup.

## Integrity and atomicity

Writing:

1. Files are streamed into an exclusively created random `.sagedock-*.sagedock-part` beside
   the destination, hashing as they go. Destinations inside workspaces are refused.
2. Each file is re-`stat`ed after reading. If its size or mtime changed, the backup is
   abandoned and the changed files are named. A copy that is half-old and half-new is worse
   than no copy.
3. The manifest is written.
4. The finished archive is **reopened and every declared SHA-256 recomputed**, and the file
   count is checked against the manifest.
5. A final scan compares the file set, sizes, and modification times with the original scan.
6. Only then is it renamed to the name the user chose.

A backup that exists at its final name is therefore one that was verified. An interrupted
backup leaves a `.sagedock-part` file. A later attempt uses a fresh random name and never
deletes a pre-existing partial file. This is not a snapshot of open, unsaved notebooks.

Restoring verifies every hash again as it extracts, and fails on the first mismatch.

## Safety when reading

Every archive is treated as hostile — one can arrive by email. Restoring refuses:

- entry paths that are absolute, name a drive (`C:`), contain `..` or `.`, contain a
  backslash (ZIP mandates forward slashes, so one means someone is being clever), are empty,
  or contain a NUL byte
- any entry whose resolved path does not remain under the destination folder
- any entry declaring a path outside the workspace it claims to belong to
- symbolic-link entries (`unix_mode & 0xF000 == 0xA000`)
- files above 8 GiB, archives above 96 GiB or 400,000 files, and any file that expands
  beyond its declared size
- manifests above 128 MiB, incorrect aggregate sizes/counts, and unsupported format versions
- Windows device names, invalid characters, trailing dots/spaces, case-insensitive duplicate
  paths/slugs, and a path used both as a file and as a directory

All workspaces are extracted to exclusively created random staging directories. After every
checksum passes, folders are published and the workspace registry is saved. A handled error
rolls back paths created by this transaction. Existing directories are never reused or
deleted. Process/power loss can leave staging or unregistered restored folders; the original
backup and pre-existing coursework remain untouched. A later restore chooses fresh names.

## Never overwriting

Restore **always adds**; it never replaces. If a workspace name is already taken
(case-insensitively, including unregistered folders on disk), the restored copy becomes `Name (restored)`, then
`Name (restored 2)`, and so on. The preview shown before restoring states the exact name
each workspace is expected to land under and flags conflicts. Concurrent filesystem changes
can require a different free name when restore actually runs. Long names are shortened to
leave room for the suffix within the 64-character workspace-name limit.

## Compatibility rules

| Situation                                  | Behaviour                                                                                                |
| ------------------------------------------ | -------------------------------------------------------------------------------------------------------- |
| `format` = 1                               | Validated and restored.                                                                                  |
| `format` = 0                               | Unsupported; refused.                                                                                    |
| `format` > 1                               | Preview still works and explains why; restore refused with `BACKUP_TOO_NEW`. The file is never modified. |
| Missing or unparseable manifest            | `BACKUP_NOT_RECOGNISED` / `BACKUP_UNREADABLE`.                                                           |
| Manifest declares a file the archive lacks | `BACKUP_DAMAGED`.                                                                                        |
| Hash mismatch                              | `BACKUP_DAMAGED`, nothing published; staging cleanup attempted.                                          |

Future format versions must keep `format` as the first decision point and must not
repurpose existing field names. Additive fields are safe: unknown keys are ignored, and
`preferences` already defaults when absent.

## Disk space

Restore recomputes and validates `total_bytes`, then requires that size rounded down to
whole GiB plus 2 GiB on the destination drive (at least 1 GiB of reserve). The check runs
before extraction; write errors still handle space consumed concurrently by other apps.
