# Third-party notices

The MIT license in [`LICENSE`](LICENSE) covers SageDock's own source code — the Windows
desktop application in `src/` and `src-tauri/`, its build scripts, and its documentation.

It does **not** cover the third-party software SageDock installs, bundles, or depends on.
Each of those components remains under its own license, held by its own copyright holders.
Nothing here modifies, replaces, or supersedes those licenses.

## Bundled scientific runtime

The SageDock installer ships a prebuilt Linux runtime image
(`sagedock-runtime-sage10.9-x64.tar.xz`). It is an assembly of independently licensed
software, including but not limited to:

| Component                                           | Typical license                           |
| --------------------------------------------------- | ----------------------------------------- |
| SageMath                                            | GPL-2.0-or-later                          |
| Python                                              | PSF License                               |
| Jupyter Server, JupyterLab, IPython, jupyter-client | BSD-3-Clause                              |
| NumPy, SciPy, pandas, scikit-learn, matplotlib      | BSD-3-Clause                              |
| SymPy                                               | BSD-3-Clause                              |
| Ubuntu base system packages                         | various (GPL, LGPL, MIT, BSD, and others) |
| Miniforge / conda-forge packages                    | various, per package                      |

The authoritative license text for every package is inside the runtime image itself, under
the usual locations (`/usr/share/doc/*/copyright` and each Python distribution's
`*.dist-info/METADATA` or `LICENSE` file). SageDock does not relicense any of it.

Optional components a user installs later through **Scientific tools** (compilers, build
tools, and additional Python packages) are likewise governed by their own licenses and are
downloaded from their upstream distributors at install time.

## Application dependencies

The desktop application is built with Tauri, Rust crates, and npm packages, each under its
own license, predominantly MIT and Apache-2.0:

| Component                          | Typical license                    |
| ---------------------------------- | ---------------------------------- |
| Tauri                              | MIT or Apache-2.0                  |
| React, React Router                | MIT                                |
| Vite, TypeScript                   | MIT or Apache-2.0                  |
| Other Rust crates and npm packages | Mostly MIT or Apache-2.0           |
| Segoe Fluent Icons                 | Included with Windows, © Microsoft |

Segoe Fluent Icons is reached through the system font stack. SageDock bundles no font file
and adds none to the installer. The complete dependency sets are recorded in
`src-tauri/Cargo.toml` / `Cargo.lock` and `package.json` / `package-lock.json`.

This attribution is also shown inside the application, under **Guides & help**, in the About
section. The two lists are meant to agree; update both together.

## Trademarks

SageMath, Jupyter, Python, Windows, and Ubuntu are trademarks of their respective owners.
SageDock is an independent project and is not affiliated with, endorsed by, or sponsored by
any of them.
