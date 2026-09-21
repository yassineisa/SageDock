# DataLab for Windows — Product & Infrastructure Specification

## 1. Product Vision

DataLab is a zero-configuration scientific computing desktop app for Windows students who need SageMath, Python, Jupyter, compilers, and common data-science tooling without having to understand WSL, Linux, package managers, shells, PATH variables, virtual environments, ports, or terminal commands.

The intended experience is closer to installing a polished consumer application than configuring a development environment.

A student should be able to:

1. Download one installer.
2. Open it.
3. Click **Install**.
4. Approve Windows permissions if required.
5. Restart only if Windows genuinely requires it.
6. Reopen DataLab.
7. Click **New Sage Notebook**.
8. Start working.

The app should hide technical infrastructure by default while still allowing advanced users to inspect diagnostics when needed.

The long-term product direction is to move from "a polished launcher for JupyterLab + SageMath" to "a first-party notebook and scientific workspace UI" built on top of the same execution infrastructure.

---

## 2. Product Principles

### 2.1 Zero Terminal Requirement

A normal user should never need to open PowerShell, Command Prompt, Windows Terminal, Bash, or a Linux shell.

Terminal access may exist under an **Advanced** menu, but no core setup, repair, package installation, compiler installation, kernel management, or notebook launch workflow should require terminal knowledge.

### 2.2 Progressive Disclosure

The app should show simple language first and technical detail only when requested.

Bad:

> WslRegisterDistribution failed with error 0x80370102.

Good:

> **Virtualization is turned off**
>
> DataLab needs a Windows feature called virtualization to run SageMath. We can check whether your computer supports it and show you exactly what to change.
>
> **Fix this**   **Learn more**   **Copy technical details**

### 2.3 Safe by Default

The app should avoid destructive actions, keep user notebooks outside the disposable Linux environment, back up configuration before migrations, and never delete user work during repair or reinstall operations.

### 2.4 Repair Instead of Blame

Errors should be presented as recoverable states.

Instead of:

> Installation failed.

Prefer:

> **DataLab could not finish setting up SageMath**
>
> Your notebooks are safe. The Linux environment stopped responding while the setup was being completed.
>
> **Try repair**   **Restart setup**   **View details**

### 2.5 One Obvious Primary Action

Every important screen should have one clear next step.

The user should never have to decide between multiple technical paths such as choosing a Linux distribution, Python environment manager, Jupyter kernel, compiler backend, package channel, or port.

### 2.6 Infrastructure Should Be Replaceable

The first release may use JupyterLab as the visible workspace, but the underlying architecture must allow DataLab to later provide its own notebook UI without replacing the execution environment.

---

## 3. High-Level Architecture

```text
┌──────────────────────────────────────────────┐
│                DataLab.exe                   │
│        Native Windows Desktop Shell          │
│                                              │
│  • Setup                                     │
│  • Health checks                             │
│  • Project browser                           │
│  • Package/compiler manager                  │
│  • Repair                                    │
│  • Updates                                   │
│  • Embedded notebook view                    │
└──────────────────────┬───────────────────────┘
                       │
                       │ Controlled local bridge
                       ▼
┌──────────────────────────────────────────────┐
│                 WSL 2                        │
│                                              │
│  Custom DataLab Linux distribution           │
│                                              │
│  • SageMath                                  │
│  • Python                                    │
│  • Jupyter Server / JupyterLab               │
│  • Scientific Python stack                   │
│  • Git                                       │
│  • Compilers                                 │
│  • Build tools                               │
│  • Kernel registrations                      │
│  • DataLab helper service                    │
└──────────────────────┬───────────────────────┘
                       │
                       ▼
┌──────────────────────────────────────────────┐
│          Windows-side user workspace         │
│                                              │
│  Documents\DataLab\                         │
│  • notebooks                                 │
│  • datasets                                  │
│  • projects                                  │
│  • exports                                   │
│                                              │
│  User work survives environment reinstall.   │
└──────────────────────────────────────────────┘
```

---

## 4. Core Components

## 4.1 Windows Desktop App

Recommended role:

- Primary UI
- Installer orchestration
- WSL detection
- WSL health checks
- Environment provisioning
- Start/stop control
- Jupyter server launch
- Embedded browser hosting
- Project selection
- Settings
- Update management
- Error display
- Diagnostics export
- Repair workflows
- Compiler/package installation UI

Recommended technology direction:

- Tauri or another lightweight native desktop shell
- WebView2 for embedded notebook display on Windows
- Native Windows dialogs where appropriate
- A small local backend responsible for privileged setup operations and process management

The Windows app should not contain scientific execution logic itself. It should orchestrate the Linux runtime.

---

## 4.2 Custom DataLab WSL Distribution

DataLab should ship a preconfigured Linux userspace rather than asking the user to manually install Ubuntu and then configure it.

The custom environment should include:

- SageMath
- Python
- Jupyter Server
- JupyterLab
- IPython
- Sage Jupyter kernel
- Python kernel
- NumPy
- SciPy
- pandas
- SymPy
- matplotlib
- scikit-learn
- Jupyter widgets support
- Git
- curl
- common archive utilities
- CA certificates
- DataLab helper scripts/services

Optional components may be installed on demand to keep the base image smaller.

The Linux environment should be treated as replaceable infrastructure, not as the canonical storage location for user work.

---

## 4.3 Windows-Side Workspace

Default location:

```text
%USERPROFILE%\Documents\DataLab\
```

Recommended structure:

```text
DataLab\
├── Projects\
├── Notebooks\
├── Datasets\
├── Exports\
└── Templates\
```

Advantages:

- User work remains visible in File Explorer.
- Reinstalling DataLab does not destroy notebooks.
- Files can be backed up by OneDrive or another backup tool.
- Users can attach files to email or upload them without understanding Linux paths.
- Repair operations can safely replace the WSL runtime.

The app may later offer a project abstraction, but it should still map to ordinary folders.

---

## 4.4 Local Runtime Bridge

DataLab should start Jupyter inside WSL and connect to it through localhost.

The bridge should:

- Start Jupyter on an automatically selected free local port.
- Generate a short-lived authentication token.
- Bind only to localhost by default.
- Never expose Jupyter to the local network unless the user explicitly enables an advanced feature.
- Detect whether the server actually started.
- Retry on port conflicts.
- Stop orphaned servers when appropriate.
- Reconnect to an already-running valid session.

The user should never need to see or enter a URL, port number, or Jupyter token.

---

# 5. User Experience

## 5.1 First Launch

The first launch should feel like setting up a consumer app.

### Screen 1 — Welcome

Example:

> **Everything you need for SageMath and scientific notebooks on Windows**
>
> DataLab sets up SageMath, Python, Jupyter, and the required Linux environment automatically.
>
> No terminal setup required.
>
> **Continue**

Secondary link:

> What will DataLab install?

This opens a friendly explanation rather than raw package names.

---

## 5.2 Preflight Check

Before making changes, DataLab should check:

- Supported Windows version
- 64-bit or ARM64 architecture
- Hardware virtualization capability
- Whether virtualization is enabled
- WSL availability
- WSL version
- WSL kernel health
- Required Windows components
- Available disk space
- Available system memory
- Internet connectivity if downloads are required
- Whether a reboot is pending
- Whether another DataLab installer is running
- Whether an existing DataLab distro exists
- Whether that distro is healthy
- File permissions for the workspace directory
- Localhost networking availability
- WebView2 availability

The screen should not dump all checks unless something requires attention.

Normal experience:

```text
Checking your computer…

✓ Windows is ready
✓ Virtualization is available
✓ Enough storage is available
✓ DataLab can create your workspace

Ready to install
```

---

## 5.3 Setup Screen

The user should see a simple progress view with human-readable stages.

Example:

```text
Setting up DataLab

✓ Preparing Windows
✓ Installing the Linux runtime
● Installing SageMath
○ Preparing Jupyter
○ Finishing setup

This only needs to happen once.
```

Avoid showing package-manager logs in the main UI.

A hidden **Show details** section may expose technical logs for advanced users or troubleshooting.

---

## 5.4 Restart Handling

If Windows requires a reboot, DataLab should explain exactly why.

Example:

> **One restart is needed**
>
> Windows needs to finish enabling the feature DataLab uses to run SageMath.
>
> Your setup progress has been saved. After you restart, DataLab will continue automatically.
>
> **Restart now**   **Restart later**

DataLab should save setup state before the reboot and resume gracefully afterward.

---

# 6. Main App Experience

The default Home screen should be intentionally simple.

Example:

```text
DataLab

[ New Sage Notebook ]
[ New Python Notebook ]

Recent
------------------------------------------------
Calculus Assignment 4
STAT 360 Project
Linear Algebra Notes

[ Open Project ]
```

SageMath should be a first-class experience, not buried under a kernel selector.

A nontechnical user should understand that Sage and Python are different notebook types without learning what a Jupyter kernel is.

---

# 7. Notebook Launch Flow

When the user clicks **New Sage Notebook**:

1. Run a lightweight health check.
2. Confirm the DataLab WSL environment is available.
3. Confirm the Sage kernel is registered.
4. Confirm the workspace is writable.
5. Start the Jupyter server if needed.
6. Wait for the server readiness endpoint.
7. Create a notebook using the Sage kernel.
8. Open the notebook in the embedded WebView or default browser.
9. If anything fails, route the failure to a specific recovery flow.

The app should not simply launch a process and hope it works.

---

# 8. Boot and Runtime Health Checks

## 8.1 Fast Check on Every Launch

Run these checks before showing the workspace as fully ready:

- WSL command responds
- DataLab distro exists
- DataLab distro starts
- Expected DataLab version marker exists
- Python responds
- Sage responds
- Jupyter responds
- Workspace path exists
- Workspace is writable
- No incompatible migration is pending

These checks should complete quickly and preferably run concurrently where safe.

---

## 8.2 Deep Check When Needed

Run a deeper diagnostic when:

- The user requests repair
- Launch repeatedly fails
- A major update was installed
- Environment corruption is suspected

Deep checks may include:

- Package integrity
- Kernel registration
- Jupyter configuration
- Sage executable availability
- Python environment consistency
- compiler availability
- DNS inside WSL
- localhost bridge
- disk usage inside the virtual disk
- filesystem permissions
- stale lock files
- stale Jupyter processes
- broken symbolic links
- runtime version mismatch

---

# 9. Error Handling Philosophy

Every error should answer four questions:

1. What happened?
2. Is my work safe?
3. Can DataLab fix it?
4. What should I do next?

Every error object should internally include:

- Friendly title
- Friendly explanation
- Severity
- Whether user files are safe
- Automated recovery actions
- Recommended primary action
- Secondary action
- Technical error code
- Original exception/process output
- Timestamp
- Component involved
- Diagnostic context

---

## 9.1 Example: WSL Missing

> **Windows needs one additional feature**
>
> DataLab uses Windows Subsystem for Linux to run SageMath. It is not installed yet.
>
> DataLab can install it for you.
>
> **Install required feature**
>
> Your files will not be affected.

---

## 9.2 Example: Virtualization Disabled

> **Virtualization is turned off**
>
> Your computer supports the technology DataLab needs, but it is currently disabled in your firmware settings.
>
> **Show me how to fix this**
>
> Technical details are available under **More information**.

For supported OEMs, DataLab can later show vendor-specific instructions.

---

## 9.3 Example: Jupyter Failed to Start

> **The notebook service did not start**
>
> DataLab tried to start Jupyter, but it stopped before becoming ready.
>
> Your notebooks are safe.
>
> **Try again**   **Repair notebook service**

---

## 9.4 Example: Corrupted Runtime

> **DataLab needs to repair its runtime**
>
> Some internal files used by SageMath are missing or damaged.
>
> Your projects and notebooks are stored separately and will not be deleted.
>
> **Repair DataLab**

Repair should reinstall the runtime without removing user work.

---

# 10. Repair System

The Repair screen should provide simple actions ordered from least destructive to most invasive.

```text
Repair DataLab

[ Restart notebook service ]
[ Restart Linux environment ]
[ Repair Sage and Jupyter ]
[ Rebuild DataLab runtime ]

Your notebooks and projects will not be removed.
```

Before a runtime rebuild:

- Stop running notebook servers.
- Save known open-document state if possible.
- Back up DataLab settings.
- Verify Windows-side project directories.
- Export relevant user-installed package metadata if supported.
- Replace or reimport the runtime.
- Restore configuration.
- Re-register kernels.
- Run a post-repair health check.

---

# 11. Compiler Installation Experience

Compiler installation must not require users to know names such as GCC, G++, Clang, build-essential, make, CMake, or Fortran unless they want advanced control.

## 11.1 Simple UI

Settings → Developer Tools

```text
Developer Tools

C/C++ Compiler
Needed by some scientific Python and Sage packages.
[ Install ]

Fortran Compiler
Needed by some numerical and scientific packages.
[ Install ]

Build Tools
Includes Make, CMake, pkg-config, and common headers.
[ Install ]

Full Scientific Build Toolkit
Recommended if a course requires compiling packages from source.
[ Install all ]
```

After installation:

```text
✓ C/C++ compiler installed
GCC 15.x
```

The exact version may remain under a details disclosure.

---

## 11.2 Smart Dependency Prompt

If a package installation fails because a compiler is missing, DataLab should detect the cause and translate it.

Instead of:

> error: command 'gcc' failed with exit status 1

Show:

> **This package needs a C/C++ compiler**
>
> DataLab can install the required compiler and try again.
>
> **Install compiler and retry**

The same concept should apply to:

- Fortran compilers
- CMake
- Rust toolchain
- Java runtime/JDK
- system headers
- database client libraries

---

# 12. Package Installation

A later polished package manager should let users search for packages without opening a terminal.

Example:

```text
Packages

Search packages…

pandas                 Installed
polars                 Install
statsmodels            Install
seaborn                Install
PyTorch                Install
```

The app should detect whether a package belongs in:

- Python
- Sage
- Linux system packages
- optional development tools

Advanced users may still use the terminal, but beginners should not need it.

---

# 13. JupyterLab Integration — Phase 1

The first production version should use JupyterLab rather than reimplement notebook functionality.

Recommended behavior:

- Launch Jupyter locally inside WSL.
- Hide token/port details.
- Prefer embedded WebView2 for an app-like experience.
- Allow an option to open notebooks in the default browser.
- Inject minimal DataLab theming only where stable and maintainable.
- Provide a native outer shell for setup, recent files, settings, package management, and repair.

JupyterLab remains responsible for:

- Notebook editing
- Code cells
- Markdown cells
- Kernel communication
- Execution output
- Rich display
- File browser
- Notebook save format

---

# 14. Custom Notebook UI — Future Architecture

DataLab should be designed so JupyterLab is a replaceable front end, not the foundation of the whole product.

The future UI should reuse the same execution layer.

Recommended future architecture:

```text
DataLab UI
    │
    ├── Notebook editor
    ├── Search
    ├── Help browser
    ├── Variable explorer
    ├── Plot viewer
    ├── File browser
    └── Package manager
          │
          ▼
Jupyter protocol / kernel gateway
          │
          ▼
Sage kernel / Python kernel
```

This allows DataLab to replace JupyterLab visually while retaining mature Jupyter kernel protocols and the `.ipynb` ecosystem.

---

# 15. Future First-Party Notebook UX

The custom DataLab notebook experience should target users who are not comfortable with traditional developer tools.

## 15.1 Search Everywhere

A global search should find:

- notebook names
- file names
- text in markdown cells
- source code
- Sage commands
- function documentation
- variables
- errors
- settings
- help topics

A user should never need to know keyboard shortcuts such as Ctrl+F to locate information.

A visible **Search** control should always be discoverable.

---

## 15.2 Help UI

The Help menu should never dump documentation into a terminal or unstructured text page.

Example:

```text
Help

Search SageMath help…

matrix
------------------------------------------------
Matrix
Create and manipulate matrices.

Examples
• Create a 3×3 matrix
• Find a determinant
• Compute eigenvalues

[ Open full guide ]
```

When the user requests help for a function, open a searchable native panel or modal.

Features:

- Search field
- Plain-language summary
- Syntax
- Examples
- Related commands
- Copy example button
- Open full documentation
- Back/forward navigation
- Search within page

---

## 15.3 Friendly Execution Errors

Where possible, interpret common Python/Sage errors.

Raw output may remain available, but the UI can prepend a plain-language explanation.

Example:

> **DataLab could not find `numpy`**
>
> This notebook is trying to use the NumPy package, but it is not installed in this environment.
>
> **Install NumPy**
>
> Technical message: `ModuleNotFoundError: No module named 'numpy'`

---

## 15.4 Visible Run Controls

Beginner-friendly notebooks should always expose obvious controls:

- Run
- Stop
- Run all
- Restart session
- Clear output
- Save

Users should not have to discover keyboard shortcuts.

---

## 15.5 Variable Explorer

Future DataLab should expose variables in a side panel.

```text
Variables
---------------------------------
x       Integer       42
A       Matrix        3 × 3
sales   DataFrame     2,481 rows
f       Function
```

Clicking a variable should open a friendly preview.

---

# 16. Updates

DataLab updates should be separated into layers.

## 16.1 Windows App Update

Updates:

- UI
- installer logic
- recovery logic
- launcher
- diagnostics

Should be small and frequent.

## 16.2 Runtime Update

Updates:

- SageMath
- Python
- Jupyter
- scientific packages
- Linux system packages

Should be versioned independently.

## 16.3 Runtime Migration Safety

Before a major runtime update:

- Confirm user files are stored outside WSL.
- Create rollback metadata.
- Save DataLab settings.
- Record optional installed components.
- Verify enough disk space.
- Install/migrate.
- Run health checks.
- Roll back automatically if validation fails.

---

# 17. Security Safeguards

Default security posture:

- Bind Jupyter only to `127.0.0.1`.
- Use a random short-lived token or equivalent local auth mechanism.
- Do not expose notebook services to the LAN.
- Do not run notebooks as Windows administrator.
- Minimize commands executed with elevation.
- Separate privileged installer work from normal app operation.
- Verify runtime package signatures/checksums where practical.
- Verify downloaded runtime manifests.
- Use HTTPS for downloads.
- Protect update metadata from tampering.
- Validate all filesystem paths before destructive operations.
- Never delete user project folders during uninstall or repair without explicit confirmation.
- Clearly distinguish DataLab-owned files from user-owned files.

---

# 18. Data Safety Safeguards

DataLab should assume that student notebooks may represent many hours of work.

Safeguards:

- Autosave enabled.
- User files stored outside the WSL runtime.
- Atomic save behavior where possible.
- Crash recovery.
- Preserve notebook checkpoints.
- Optional local version history later.
- Warn before overwriting an existing notebook.
- Never remove user project directories during runtime repair.
- Offer a backup/export option before major migrations.

---

# 19. Diagnostics

The user-facing diagnostics experience should be simple.

Settings → Help → Diagnostics

```text
DataLab status

Windows integration      ✓
Linux runtime            ✓
SageMath                 ✓
Python                   ✓
Jupyter                  ✓
Workspace                ✓
Networking               ✓

[ Run full diagnostic ]
[ Copy diagnostic report ]
[ Save report ]
```

The exported diagnostic report may contain technical details such as:

- DataLab version
- runtime version
- Windows version
- WSL version
- architecture
- Sage version
- Python version
- Jupyter version
- detected compilers
- failed checks
- sanitized recent logs

Diagnostics must avoid including notebook contents, passwords, authentication tokens, or unrelated personal files.

---

# 20. Logging

Use structured logs rather than a single opaque text file.

Suggested categories:

- installer
- launcher
- WSL
- runtime
- Jupyter
- kernel
- package manager
- updates
- repair

Log levels:

- info
- warning
- error
- debug

Debug logs should be opt-in for normal production users if they may include additional system details.

---

# 21. Startup State Machine

DataLab should use an explicit state machine rather than scattered startup logic.

Suggested states:

```text
APP_START
    ↓
CHECK_WINDOWS
    ↓
CHECK_WSL
    ↓
CHECK_DATALAB_DISTRO
    ↓
CHECK_RUNTIME_VERSION
    ↓
CHECK_WORKSPACE
    ↓
CHECK_JUPYTER
    ↓
READY
```

Possible recovery branches:

```text
WSL_MISSING           → INSTALL_WSL
WSL_REBOOT_REQUIRED   → REQUEST_REBOOT
DISTRO_MISSING        → INSTALL_RUNTIME
DISTRO_BROKEN         → REPAIR_RUNTIME
MIGRATION_REQUIRED    → MIGRATE_RUNTIME
JUPYTER_BROKEN        → REPAIR_JUPYTER
WORKSPACE_UNWRITABLE  → REPAIR_PERMISSIONS
PORT_FAILURE          → SELECT_NEW_PORT
```

An explicit state machine makes recovery predictable and prevents "half-installed" ambiguous states.

---

# 22. Installer State Machine

Suggested phases:

1. `PREFLIGHT`
2. `ENABLE_WINDOWS_FEATURES`
3. `WAITING_FOR_REBOOT` if needed
4. `INSTALL_WSL`
5. `IMPORT_DATALAB_RUNTIME`
6. `CONFIGURE_RUNTIME`
7. `CREATE_WORKSPACE`
8. `REGISTER_KERNELS`
9. `START_JUPYTER_TEST`
10. `VERIFY_SAGE_TEST`
11. `VERIFY_PYTHON_TEST`
12. `FINALIZE`
13. `READY`

Each stage should be idempotent where practical, allowing setup to resume after interruption.

---

# 23. Installation Verification

Before presenting **Setup complete**, DataLab should actually execute tests.

Minimum verification:

### Sage test

Run a small Sage command and verify expected output.

Example concept:

```text
factor(123456)
```

### Python test

Run Python and import a small standard set of packages.

### Jupyter test

Start Jupyter and confirm a local health endpoint responds.

### Kernel test

Create or launch a temporary notebook kernel and confirm it executes a trivial cell.

### Workspace test

Create, write, read, and delete a temporary test file in the DataLab workspace.

Only after these checks pass should setup report success.

---

# 24. Offline and Limited Connectivity Behavior

Where possible, the installer should support either:

- a compact online installer that downloads the runtime, or
- a larger offline installer containing the runtime image.

If a download fails:

> **The download was interrupted**
>
> DataLab saved the parts that were downloaded successfully. Check your internet connection and continue when you are ready.
>
> **Resume download**

Support resumable downloads where practical.

---

# 25. Low Disk Space Handling

Before installation or update, estimate required storage with a safety margin.

Example:

> **DataLab needs more free space**
>
> 7.4 GB is available, but this installation needs about 11 GB.
>
> Free at least 4 GB and try again.
>
> **Open Storage settings**

Do not begin a major runtime migration if available space is too low for rollback.

---

# 26. Uninstall Behavior

Uninstalling DataLab should clearly separate the application from user work.

Suggested uninstall choices:

```text
Remove DataLab

✓ Remove DataLab application
✓ Remove Linux runtime

□ Also remove my DataLab projects and notebooks

Your project files will be kept unless you choose the last option.
```

Default: preserve user work.

---

# 27. Accessibility

Apple-level polish should include accessibility, not only visual refinement.

Requirements:

- Full keyboard navigation
- Screen-reader labels
- Proper focus order
- High-contrast compatibility
- Scalable text
- No information conveyed by color alone
- Reduced-motion support
- Clear loading states
- Large click targets
- Plain-language errors

---

# 28. Visual and Interaction Standard

The product should feel calm and deliberate.

Guidelines:

- Avoid dense dashboards.
- Avoid terminal-themed visuals for beginners.
- Avoid exposing infrastructure vocabulary unless needed.
- Prefer short sentences.
- Use clear status indicators.
- Keep primary actions visually dominant.
- Provide reversible actions.
- Preserve context when opening help.
- Show progress during operations longer than a moment.
- Never leave the UI looking frozen.

---

# 29. Suggested Settings Structure

```text
Settings

General
• Open notebooks in DataLab / browser
• Default notebook type
• Default project folder

Appearance
• System / Light / Dark
• Editor font size

Scientific Tools
• SageMath status
• Python status
• Installed packages
• Developer tools / compilers

Storage
• Workspace location
• Runtime disk usage
• Clear temporary files

Updates
• App updates
• Runtime updates
• Update channel

Advanced
• Open Linux shell
• Jupyter settings
• Network settings
• Reset runtime

Help
• Search help
• Diagnostics
• Export diagnostic report
• About DataLab
```

---

# 30. MVP Deliverables

## Phase 1 — Functional Prototype

Deliverables:

- Windows desktop launcher
- WSL detection
- DataLab runtime import
- SageMath installed
- Python installed
- JupyterLab installed
- Sage kernel registered
- Python kernel registered
- Windows workspace folder
- Start/stop Jupyter
- Open Jupyter in browser
- Basic health checks
- Basic error handling

---

## Phase 2 — Polished Student Release

Deliverables:

- Single friendly installer
- Setup state machine
- reboot resume flow
- embedded WebView2 notebook mode
- recent notebooks
- new Sage notebook button
- new Python notebook button
- repair center
- diagnostics center
- runtime update system
- compiler installer UI
- package management UI
- robust logging
- low-storage handling
- network/download recovery
- safe uninstall flow

---

## Phase 3 — Premium UX Layer

Deliverables:

- project management
- file previews
- starter templates
- friendlier error explanations
- searchable documentation panel
- help popovers
- compiler auto-detection
- dependency resolution prompts
- notebook recovery UI
- environment snapshots

---

## Phase 4 — First-Party Notebook UI

Deliverables:

- DataLab notebook editor
- `.ipynb` compatibility
- Jupyter kernel protocol client
- Sage/Python kernel management
- cell execution
- rich output rendering
- searchable help
- global search
- variable explorer
- plot viewer
- friendly package install prompts
- first-party file browser
- beginner-first menus
- visible notebook actions

At this stage, JupyterLab becomes optional rather than the primary UX.

---

# 31. Non-Goals for Early Versions

Do not initially build:

- a custom Linux kernel
- a custom hypervisor
- a full package resolver
- a replacement for Jupyter kernels
- a custom notebook file format
- a custom Python distribution
- a custom SageMath fork
- remote cloud execution
- collaborative notebooks

The app should win by packaging mature infrastructure beautifully, not by rewriting proven components.

---

# 32. Success Criteria

The project is successful when a first-year university student with minimal computer experience can:

- install the app without reading a technical guide
- create a Sage notebook
- run Sage code
- create a Python notebook
- open an existing notebook
- install a compiler when prompted
- install a common package without a terminal
- understand what went wrong when an error occurs
- repair the app without losing coursework
- find help without using the terminal or knowing keyboard shortcuts

A useful benchmark:

> If the product requires a user to Google "how to use WSL" or "how to open Bash," the product has failed its primary UX goal.

---

# 33. Final Product Promise

DataLab should feel like this:

> Install one Windows app. Open it. Choose SageMath or Python. Start working.

Everything else — Linux, WSL, Jupyter servers, compilers, ports, package managers, kernels, configuration files, and recovery commands — is infrastructure DataLab manages on the user's behalf.

The long-term ambition is not merely to hide Linux. It is to provide a scientific computing environment that feels designed for students first, with enough maturity underneath to remain useful as those students become more advanced.
