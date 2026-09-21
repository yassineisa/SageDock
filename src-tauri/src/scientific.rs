//! Compilers, build tools, and optional Python packages inside SageDock's own runtime.
//!
//! Two rules shape this module.
//!
//! **The frontend never names a package or a command.** It sends a `Tool` enum variant and
//! nothing else. Every package name, every probe program, and every shell script here is a
//! fixed constant, so there is no path by which the webview can install arbitrary software
//! or run arbitrary code. Dependencies are resolved by the runtime package manager.
//!
//! **A command on `PATH` is not proof that a compiler works.** A half-installed `gcc` with
//! no C library headers, a `g++` whose standard library is missing, or a `cmake` that cannot
//! configure a project all answer `--version` perfectly happily. So every component is
//! verified by *compiling and running a small program* inside the runtime SageDock actually
//! uses. That is the difference between "installed" and "works", and both are reported.

use std::path::Path;

use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::{
    error::{AppError, AppResult},
    runtime::wsl,
};

// --- what can be installed ----------------------------------------------------------------

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    Cpp,
    Fortran,
    Build,
    /// Everything above, installed in one transaction.
    Toolkit,
    Seaborn,
    Statsmodels,
    Polars,
}

/// One executable that has to exist *and* work for a tool to count as installed.
///
/// Split out because "build tools" is three independent programs, and a student whose
/// `cmake` works but whose `make` is missing needs to be told exactly that rather than
/// being shown a single misleading tick.
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Component {
    Gcc,
    Gxx,
    Gfortran,
    Make,
    Cmake,
    PkgConfig,
}

impl Component {
    /// The apt package providing it. This list *is* the allowlist.
    fn package(self) -> &'static str {
        match self {
            Self::Gcc => "gcc",
            Self::Gxx => "g++",
            Self::Gfortran => "gfortran",
            Self::Make => "make",
            Self::Cmake => "cmake",
            Self::PkgConfig => "pkg-config",
        }
    }

    /// Plain language, with the command in brackets for anyone who wants it.
    fn label(self) -> &'static str {
        match self {
            Self::Gcc => "C compiler (gcc)",
            Self::Gxx => "C++ compiler (g++)",
            Self::Gfortran => "Fortran compiler (gfortran)",
            Self::Make => "Make",
            Self::Cmake => "CMake",
            Self::PkgConfig => "pkg-config",
        }
    }
}

impl Tool {
    /// Components this tool requires. Empty for the Python packages, which are verified by
    /// importing them instead.
    pub fn components(self) -> &'static [Component] {
        match self {
            Self::Cpp => &[Component::Gcc, Component::Gxx],
            Self::Fortran => &[Component::Gfortran],
            Self::Build => &[Component::Make, Component::Cmake, Component::PkgConfig],
            Self::Toolkit => &[
                Component::Gcc,
                Component::Gxx,
                Component::Gfortran,
                Component::Make,
                Component::Cmake,
                Component::PkgConfig,
            ],
            _ => &[],
        }
    }

    fn is_system(self) -> bool {
        !self.components().is_empty()
    }

    /// The Python module name, for the additive package installs.
    fn module(self) -> Option<&'static str> {
        match self {
            Self::Seaborn => Some("seaborn"),
            Self::Statsmodels => Some("statsmodels"),
            Self::Polars => Some("polars"),
            _ => None,
        }
    }
}

// --- what is reported back ------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolState {
    Installed,
    /// Present but not working, or only partly present. Repair installs what is missing.
    NeedsRepair,
    NotInstalled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentStatus {
    pub id: Component,
    /// Owned rather than `&'static str` because this round-trips through the on-disk cache,
    /// and deriving `Deserialize` for a borrowed field would tie the whole report to
    /// `'static`.
    pub label: String,
    /// Found on `PATH`. Necessary, never sufficient.
    pub present: bool,
    /// Compiled and ran a test program, or performed its real job.
    pub works: bool,
    pub version: Option<String>,
    /// Why it did not work, when it did not.
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolStatus {
    pub id: Tool,
    pub state: ToolState,
    pub version: Option<String>,
    pub components: Vec<ComponentStatus>,
}

/// Where the numbers came from. The UI must never present `Cached` or `Unavailable` as
/// though it were a fresh, negative answer.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportSource {
    /// Probed just now inside the running environment.
    Verified,
    /// Last known good result, read from disk. The environment was not woken to get it.
    Cached,
    /// Nothing is known. **Not** the same as "not installed".
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolReport {
    pub source: ReportSource,
    /// When the underlying probe ran, RFC 3339. `None` when nothing is known.
    pub checked_at: Option<String>,
    /// Plain-language explanation when `source` is not `Verified`.
    pub reason: Option<String>,
    pub tools: Vec<ToolStatus>,
}

// --- progress ---------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolStage {
    Checking,
    Installing,
    Verifying,
    Done,
}

#[derive(Clone, Debug, Serialize)]
pub struct ToolProgress {
    pub stage: ToolStage,
    pub title: String,
    pub detail: Option<String>,
    pub percent: Option<f32>,
}

impl ToolProgress {
    fn new(stage: ToolStage, title: &str, detail: Option<String>, percent: Option<f32>) -> Self {
        Self {
            stage,
            title: title.to_string(),
            detail,
            percent,
        }
    }
}

pub type ProgressSink<'a> = &'a mut dyn FnMut(ToolProgress);

// --- pure decision logic (unit tested) ----------------------------------------------------

/// Turns component results into one state.
///
/// The middle case is the one that matters: something present but not working, or only some
/// of a group installed, is `NeedsRepair`, never `Installed` and never `NotInstalled`. A
/// student whose `make` is missing but whose `cmake` works must not be told to install from
/// scratch, nor told everything is fine.
fn state_of(components: &[ComponentStatus]) -> ToolState {
    if components.is_empty() {
        return ToolState::NotInstalled;
    }
    if components.iter().all(|c| c.works) {
        ToolState::Installed
    } else if components.iter().any(|c| c.works || c.present) {
        ToolState::NeedsRepair
    } else {
        ToolState::NotInstalled
    }
}

/// Packages still needed for `tool`, given what already works.
///
/// Anything already verified is skipped, so "Install all" on a machine that already has a C
/// compiler does not re-fetch it, and a repair installs only the missing piece.
fn missing_packages(tool: Tool, components: &[ComponentStatus]) -> Vec<&'static str> {
    tool.components()
        .iter()
        .filter(|required| {
            !components
                .iter()
                .any(|status| status.id == **required && status.works)
        })
        .map(|component| component.package())
        .collect()
}

/// Why an apt run failed, in terms a student can act on.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum AptFailure {
    Busy,
    Network,
    Storage,
    Other,
}

/// Classifies apt output. Matching is lowercase and substring-based because apt's wording
/// varies by version, and the consequence of a wrong guess is only which advice is shown.
fn classify_apt_failure(output: &str) -> AptFailure {
    let text = output.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| text.contains(n));

    if has(&[
        "could not get lock",
        "database lock was locked",
        "frontend lock was locked",
        "unable to acquire the dpkg frontend lock",
        "another process is using",
        "is another process using it",
    ]) {
        AptFailure::Busy
    } else if has(&[
        "no space left on device",
        "you don't have enough free space",
        "not enough free disk space",
        "write error",
    ]) {
        AptFailure::Storage
    } else if has(&[
        "temporary failure resolving",
        "could not resolve",
        "failed to fetch",
        "network is unreachable",
        "connection timed out",
        "no route to host",
    ]) {
        AptFailure::Network
    } else {
        AptFailure::Other
    }
}

fn apt_error(failure: AptFailure, details: String) -> AppError {
    let (code, title, message) = match failure {
        AptFailure::Busy => (
            "TOOL_PACKAGE_MANAGER_BUSY",
            "Something else is installing software right now",
            "Another installation inside the computing environment holds the software manager. Wait a minute and try again. Your saved notebooks are safe.",
        ),
        AptFailure::Network => (
            "TOOL_NETWORK_FAILED",
            "SageDock couldn't download the compiler",
            "The download didn't finish. Check your internet connection, then try again. Some installation steps may have completed. Retry to finish and verify the tool. Your saved notebooks are safe.",
        ),
        AptFailure::Storage => (
            "TOOL_OUT_OF_SPACE",
            "There isn't enough storage to install this",
            "Free some space on this PC and try again. Some installation steps may have completed. Retry to finish and verify the tool. Your saved notebooks are safe.",
        ),
        AptFailure::Other => (
            "TOOL_INSTALL_FAILED",
            "This scientific tool needs attention",
            "Your notebooks are safe. Check your internet connection and available storage, then use Repair to try again. SageDock checks the tool before marking it installed.",
        ),
    };
    AppError::new("tools", code, title, message).with_technical_details(details)
}

fn failed(details: impl Into<String>) -> AppError {
    apt_error(AptFailure::Other, details.into())
}

// --- probing the runtime ----------------------------------------------------------------------

/// Verifies every component by building and running something with it.
///
/// Each check is independent and wrapped, so one missing compiler cannot stop the others
/// being reported. The build tools deliberately avoid needing a compiler: CMake configures a
/// `NONE`-language project, Make runs a shell rule, and pkg-config resolves a `.pc` file
/// written here. That keeps "is CMake working" a question about CMake.
const PROBE: &str = r#"import json, os, shutil, subprocess, sys, tempfile

TOKEN = 'SAGEDOCK_OK'

def run(cmd, cwd=None, env=None, timeout=90):
    try:
        done = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout)
        return done.returncode, (done.stdout or '') + (done.stderr or '')
    except Exception as error:
        return 127, str(error)

def version_of(command):
    code, out = run([command, '--version'], timeout=20)
    if code != 0:
        return None
    for line in out.splitlines():
        if line.strip():
            return line.strip()[:120]
    return None

def write(path, text):
    with open(path, 'w') as handle:
        handle.write(text)

results = []

def record(cid, command, works, detail):
    results.append(dict(id=cid, present=bool(shutil.which(command)), works=bool(works),
                        version=version_of(command) if shutil.which(command) else None,
                        detail=(detail or None)))

work = tempfile.mkdtemp(prefix='sagedock-probe-')
try:
    # --- C: compile and run ---
    detail = ''
    ok = False
    try:
        if shutil.which('gcc'):
            src = os.path.join(work, 'probe.c')
            binary = os.path.join(work, 'probe_c')
            write(src, '#include <stdio.h>\nint main(void){printf("%s\\n", "' + TOKEN + '");return 0;}\n')
            code, out = run(['gcc', src, '-o', binary], cwd=work)
            if code != 0:
                detail = 'compile failed: ' + out.strip()[:300]
            else:
                code, out = run([binary], cwd=work)
                ok = code == 0 and TOKEN in out
                if not ok:
                    detail = 'compiled program did not run: ' + out.strip()[:300]
        else:
            detail = 'gcc was not found'
    except Exception as error:
        detail = str(error)[:300]
    record('gcc', 'gcc', ok, detail)

    # --- C++: needs the standard library, not just the driver ---
    detail = ''
    ok = False
    try:
        if shutil.which('g++'):
            src = os.path.join(work, 'probe.cpp')
            binary = os.path.join(work, 'probe_cpp')
            write(src, '#include <iostream>\n#include <string>\nint main(){std::string s="' + TOKEN + '";auto f=[&]{return s;};std::cout<<f()<<std::endl;return 0;}\n')
            code, out = run(['g++', src, '-o', binary], cwd=work)
            if code != 0:
                detail = 'compile failed: ' + out.strip()[:300]
            else:
                code, out = run([binary], cwd=work)
                ok = code == 0 and TOKEN in out
                if not ok:
                    detail = 'compiled program did not run: ' + out.strip()[:300]
        else:
            detail = 'g++ was not found'
    except Exception as error:
        detail = str(error)[:300]
    record('gxx', 'g++', ok, detail)

    # --- Fortran ---
    detail = ''
    ok = False
    try:
        if shutil.which('gfortran'):
            src = os.path.join(work, 'probe.f90')
            binary = os.path.join(work, 'probe_f')
            write(src, 'program probe\n  print *, "' + TOKEN + '"\nend program probe\n')
            code, out = run(['gfortran', src, '-o', binary], cwd=work)
            if code != 0:
                detail = 'compile failed: ' + out.strip()[:300]
            else:
                code, out = run([binary], cwd=work)
                ok = code == 0 and TOKEN in out
                if not ok:
                    detail = 'compiled program did not run: ' + out.strip()[:300]
        else:
            detail = 'gfortran was not found'
    except Exception as error:
        detail = str(error)[:300]
    record('gfortran', 'gfortran', ok, detail)

    # --- Make: a shell rule, so no compiler is required to judge Make ---
    detail = ''
    ok = False
    try:
        if shutil.which('make'):
            makedir = os.path.join(work, 'mk')
            os.makedirs(makedir, exist_ok=True)
            write(os.path.join(makedir, 'Makefile'), 'all:\n\t@echo ' + TOKEN + '\n')
            code, out = run(['make', '-s', 'all'], cwd=makedir)
            ok = code == 0 and TOKEN in out
            if not ok:
                detail = 'make did not run its rule: ' + out.strip()[:300]
        else:
            detail = 'make was not found'
    except Exception as error:
        detail = str(error)[:300]
    record('make', 'make', ok, detail)

    # --- CMake: configure a NONE-language project ---
    detail = ''
    ok = False
    try:
        if shutil.which('cmake'):
            source = os.path.join(work, 'cm')
            build = os.path.join(work, 'cm-build')
            os.makedirs(source, exist_ok=True)
            write(os.path.join(source, 'CMakeLists.txt'),
                  'cmake_minimum_required(VERSION 3.10)\nproject(sagedock_probe NONE)\n')
            code, out = run(['cmake', '-S', source, '-B', build], cwd=work, timeout=120)
            ok = code == 0
            if not ok:
                detail = 'cmake could not configure a project: ' + out.strip()[:300]
        else:
            detail = 'cmake was not found'
    except Exception as error:
        detail = str(error)[:300]
    record('cmake', 'cmake', ok, detail)

    # --- pkg-config: resolve a .pc file written here ---
    detail = ''
    ok = False
    try:
        if shutil.which('pkg-config'):
            pcdir = os.path.join(work, 'pc')
            os.makedirs(pcdir, exist_ok=True)
            write(os.path.join(pcdir, 'sagedock_probe.pc'),
                  'Name: sagedock_probe\nDescription: SageDock probe\nVersion: 1.2.3\n')
            env = dict(os.environ)
            env['PKG_CONFIG_PATH'] = pcdir
            code, out = run(['pkg-config', '--modversion', 'sagedock_probe'], cwd=work, env=env)
            ok = code == 0 and '1.2.3' in out
            if not ok:
                detail = 'pkg-config could not resolve a package file: ' + out.strip()[:300]
        else:
            detail = 'pkg-config was not found'
    except Exception as error:
        detail = str(error)[:300]
    record('pkg_config', 'pkg-config', ok, detail)
finally:
    shutil.rmtree(work, ignore_errors=True)

# --- optional Python packages: importing is the real test ---
modules = []
for name in ['seaborn', 'statsmodels', 'polars']:
    try:
        import importlib
        module = importlib.import_module(name)
        modules.append(dict(id=name, installed=True, version=getattr(module, '__version__', 'Installed')))
    except Exception:
        modules.append(dict(id=name, installed=False, version=None))

print('SAGEDOCK_TOOLS=' + json.dumps(dict(components=results, modules=modules)))"#;

#[derive(Deserialize)]
struct RawComponent {
    id: Component,
    present: bool,
    works: bool,
    version: Option<String>,
    detail: Option<String>,
}

#[derive(Deserialize)]
struct RawModule {
    id: Tool,
    installed: bool,
    version: Option<String>,
}

#[derive(Deserialize)]
struct RawProbe {
    components: Vec<RawComponent>,
    modules: Vec<RawModule>,
}

/// Assembles the report from raw probe output. Pure, so the shape is unit-testable.
fn assemble(raw: RawProbe, checked_at: String) -> AppResult<ToolReport> {
    // A partial or contradictory response cannot establish that a complete toolchain works.
    if raw.components.len() != Tool::Toolkit.components().len()
        || Tool::Toolkit
            .components()
            .iter()
            .any(|id| raw.components.iter().filter(|c| c.id == *id).count() != 1)
        || raw.components.iter().any(|c| c.works && !c.present)
        || raw.modules.len() != 3
        || [Tool::Seaborn, Tool::Statsmodels, Tool::Polars]
            .iter()
            .any(|id| raw.modules.iter().filter(|m| m.id == *id).count() != 1)
    {
        return Err(failed(
            "The tool check returned an incomplete or inconsistent result",
        ));
    }
    let components: Vec<ComponentStatus> = raw
        .components
        .into_iter()
        .map(|c| ComponentStatus {
            label: c.id.label().to_string(),
            id: c.id,
            present: c.present,
            works: c.works,
            version: c.version,
            detail: c.detail,
        })
        .collect();

    let mut tools = Vec::new();
    for tool in [Tool::Cpp, Tool::Fortran, Tool::Build, Tool::Toolkit] {
        let mine: Vec<ComponentStatus> = tool
            .components()
            .iter()
            .filter_map(|id| components.iter().find(|c| c.id == *id).cloned())
            .collect();
        // The headline version is the first working component's, which is the one a student
        // would quote ("GCC 13.2"). Details carry the rest.
        let version = mine
            .iter()
            .find(|c| c.works)
            .and_then(|c| c.version.clone());
        tools.push(ToolStatus {
            id: tool,
            state: state_of(&mine),
            version,
            components: mine,
        });
    }

    for module in raw.modules {
        tools.push(ToolStatus {
            id: module.id,
            state: if module.installed {
                ToolState::Installed
            } else {
                ToolState::NotInstalled
            },
            version: module.version,
            components: Vec::new(),
        });
    }

    Ok(ToolReport {
        source: ReportSource::Verified,
        checked_at: Some(checked_at),
        reason: None,
        tools,
    })
}

// --- cache -------------------------------------------------------------------------------------

/// The last verified result, so Home can show capabilities without waking a stopped
/// environment. Cached values are always labelled as such in the UI.
#[derive(Serialize, Deserialize)]
struct CachedReport {
    checked_at: String,
    tools: Vec<ToolStatus>,
}

fn cache_path(data: &Path) -> std::path::PathBuf {
    data.join("tool-status.json")
}

fn read_cache(data: &Path) -> Option<CachedReport> {
    let bytes = std::fs::read(cache_path(data)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_cache(data: &Path, report: &ToolReport) {
    let Some(checked_at) = report.checked_at.clone() else {
        return;
    };
    let cached = CachedReport {
        checked_at,
        tools: report.tools.clone(),
    };
    if let Ok(bytes) = serde_json::to_vec(&cached) {
        if let Err(err) = crate::storage::atomic_write(&cache_path(data), &bytes) {
            tracing::warn!(target: "tools", %err, "could not cache the tool status");
        }
    }
}

/// Discards the cache. Called when the runtime is replaced, because the new environment's
/// compilers have nothing to do with the old one's.
pub fn forget_cache(data: &Path) {
    let path = cache_path(data);
    if path.exists() {
        if let Err(err) = std::fs::remove_file(&path) {
            tracing::warn!(target: "tools", %err, "could not clear the tool status cache");
        }
    }
}

fn unavailable(data: &Path, reason: &str) -> ToolReport {
    match read_cache(data) {
        Some(cached) => ToolReport {
            source: ReportSource::Cached,
            checked_at: Some(cached.checked_at),
            reason: Some(reason.to_string()),
            tools: cached.tools,
        },
        None => ToolReport {
            source: ReportSource::Unavailable,
            checked_at: None,
            reason: Some(reason.to_string()),
            tools: Vec::new(),
        },
    }
}

// --- public API -------------------------------------------------------------------------------

/// Probes the runtime and refreshes the cache. Starts the environment if it is stopped, so
/// callers that must not do that go through [`report`] instead.
pub fn verify_now(data: &Path) -> AppResult<ToolReport> {
    wsl::ensure_owned(data)?;
    let result = wsl::run_as_timeout(
        wsl::LINUX_USER,
        &["/opt/sagedock/bin/sagedock-env", "python", "-c", PROBE],
        300,
    )?;

    if !result.success {
        return Err(failed(result.combined_output));
    }
    let line = result
        .combined_output
        .lines()
        .find_map(|l| l.strip_prefix("SAGEDOCK_TOOLS="))
        .ok_or_else(|| failed("The tool check did not report a result"))?;

    let raw: RawProbe = serde_json::from_str(line).map_err(|e| failed(e.to_string()))?;
    let checked_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default();
    let report = assemble(raw, checked_at)?;
    write_cache(data, &report);
    Ok(report)
}

/// Passive reads are cache-only, even while WSL runs: Home must not compile programs or
/// race with Stop. Explicit checks must hold the application operation guard.
pub fn report(data: &Path, force: bool) -> ToolReport {
    if !wsl::distro_exists(wsl::distro_name()) || wsl::ensure_owned(data).is_err() {
        return ToolReport {
            source: ReportSource::Unavailable,
            checked_at: None,
            reason: Some("SageDock's computing environment isn't available. Open Home to finish setup or Recovery to check it.".into()),
            tools: Vec::new(),
        };
    }

    if !force {
        return unavailable(data, "Choose Check now to verify the tools. Viewing this screen does not start SageMath or run compiler checks.");
    }

    match verify_now(data) {
        Ok(report) => report,
        Err(err) => {
            tracing::warn!(target: "tools", code = %err.code, "tool check failed");
            unavailable(
                data,
                "SageDock couldn't check the installed tools just now. This is not the same as them being missing.",
            )
        }
    }
}

/// Installs a tool, skipping anything already verified, then proves the result.
///
/// There is deliberately no cancel: interrupting apt mid-transaction is how a package
/// database ends up half-written. The operation lock in `AppState` already prevents a
/// second install starting alongside this one.
pub fn install(tool: Tool, data: &Path, emit: ProgressSink<'_>) -> AppResult<ToolReport> {
    wsl::ensure_owned(data)?;
    emit(ToolProgress::new(
        ToolStage::Checking,
        "Checking what's already installed",
        Some("Nothing already working will be downloaded again.".into()),
        None,
    ));

    if tool.is_system() {
        // A fresh probe first, so "install all" on a machine that already has a C compiler
        // installs only what is genuinely missing.
        let before = verify_now(data)?;
        let components = before
            .tools
            .iter()
            .flat_map(|t| t.components.iter().cloned())
            .collect::<Vec<_>>();
        let packages = missing_packages(tool, &components);

        if packages.is_empty() {
            emit(ToolProgress::new(
                ToolStage::Done,
                "Already installed",
                None,
                Some(1.0),
            ));
            return Ok(before);
        }

        emit(ToolProgress::new(
            ToolStage::Installing,
            "Downloading and installing",
            Some(format!(
                "Installing {}. This can take several minutes and shouldn't be interrupted.",
                packages.join(", ")
            )),
            None,
        ));

        // Invalidate before mutation so a partial transaction cannot retain a success.
        forget_cache(data);
        let mut args = vec![
            "bash",
            "-c",
            include_str!("tool-install.sh"),
            "sagedock-tools",
        ];
        args.extend(packages.iter().copied());
        let result = wsl::run_as_timeout("root", &args, 1800)?;

        if !result.success {
            return Err(apt_error(
                classify_apt_failure(&result.combined_output),
                result.combined_output,
            ));
        }
    } else {
        let module = tool
            .module()
            .ok_or_else(|| failed("That tool cannot be installed"))?;

        emit(ToolProgress::new(
            ToolStage::Installing,
            "Downloading and installing",
            Some("Optional packages are kept separate from SageMath's own libraries.".into()),
            None,
        ));

        // Additive only: installed into its own directory with `--no-deps`, and published
        // to Python via a `.pth` file *after* it imports cleanly. Nothing SageMath ships is
        // upgraded or replaced.
        forget_cache(data);
        let code = r#"import pathlib, subprocess, sys, tempfile, site, os
name=sys.argv[1]
packages={'seaborn':['seaborn'], 'statsmodels':['statsmodels','patsy'], 'polars':['polars','polars-runtime-32']}[name]
base=pathlib.Path.home()/'.sagedock-packages'; base.mkdir(exist_ok=True)
target=pathlib.Path(tempfile.mkdtemp(prefix=name+'-',dir=base))
subprocess.run([sys.executable,'-m','pip','install','--disable-pip-version-check','--only-binary=:all:','--no-deps','--target',str(target),*packages],check=True,timeout=900)
check='import sys; sys.path.insert(0,sys.argv[1]); __import__(sys.argv[2])'
subprocess.run([sys.executable,'-c',check,str(target),name],check=True,timeout=60)
user=pathlib.Path(site.getusersitepackages()); user.mkdir(parents=True,exist_ok=True)
temp=user/('sagedock-'+name+'.pth.tmp'); temp.write_text(str(target)+'\n')
os.replace(temp,user/('sagedock-'+name+'.pth'))
print('INSTALLED')"#;

        let result = wsl::run_as_timeout(
            wsl::LINUX_USER,
            &[
                "/opt/sagedock/bin/sagedock-env",
                "python",
                "-c",
                code,
                module,
            ],
            1100,
        )?;
        if !result.success {
            return Err(apt_error(
                classify_apt_failure(&result.combined_output),
                result.combined_output,
            ));
        }
    }

    emit(ToolProgress::new(
        ToolStage::Verifying,
        "Checking it works",
        Some(
            if tool.is_system() {
                "SageDock checks each compiler and build tool before reporting success."
            } else {
                "SageDock checks that the package can be used in Python before reporting success."
            }
            .into(),
        ),
        None,
    ));

    let after = verify_now(data)?;
    let installed = after
        .tools
        .iter()
        .find(|t| t.id == tool)
        .map(|t| t.state == ToolState::Installed)
        .unwrap_or(false);

    if !installed {
        let detail = after
            .tools
            .iter()
            .find(|t| t.id == tool)
            .map(|t| {
                t.components
                    .iter()
                    .filter(|c| !c.works)
                    .map(|c| format!("{}: {}", c.label, c.detail.clone().unwrap_or_default()))
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .unwrap_or_default();
        return Err(AppError::new(
            "tools",
            "TOOL_VERIFICATION_FAILED",
            "That tool installed but didn't pass its check",
            "The installation finished, but the tool did not pass its functional check. Your saved notebooks are safe. Use Repair to try again, or open Recovery to check the environment.",
        )
        .with_technical_details(detail));
    }

    emit(ToolProgress::new(ToolStage::Done, "Ready", None, Some(1.0)));
    Ok(after)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(id: Component, present: bool, works: bool) -> ComponentStatus {
        ComponentStatus {
            id,
            label: id.label().to_string(),
            present,
            works,
            version: works.then(|| "13.2.0".to_string()),
            detail: None,
        }
    }

    // --- state -------------------------------------------------------------------------

    #[test]
    fn every_component_working_is_installed() {
        let components = vec![
            component(Component::Gcc, true, true),
            component(Component::Gxx, true, true),
        ];
        assert_eq!(state_of(&components), ToolState::Installed);
    }

    /// The reported bug class: a group is only partly installed. Reporting that as
    /// "Installed" hides a missing program; reporting it as "Not installed" would make the
    /// student reinstall what they already have.
    #[test]
    fn a_partly_installed_group_needs_repair() {
        let components = vec![
            component(Component::Make, true, true),
            component(Component::Cmake, false, false),
            component(Component::PkgConfig, false, false),
        ];
        assert_eq!(state_of(&components), ToolState::NeedsRepair);
    }

    /// Present on PATH but unable to build anything is exactly the case a `which` check
    /// gets wrong, so it must not read as installed.
    #[test]
    fn present_but_not_working_needs_repair() {
        let components = vec![component(Component::Gcc, true, false)];
        assert_eq!(state_of(&components), ToolState::NeedsRepair);
    }

    #[test]
    fn nothing_present_is_not_installed() {
        let components = vec![
            component(Component::Gcc, false, false),
            component(Component::Gxx, false, false),
        ];
        assert_eq!(state_of(&components), ToolState::NotInstalled);
    }

    // --- choosing packages ---------------------------------------------------------------

    #[test]
    fn a_working_component_is_not_reinstalled() {
        let components = vec![
            component(Component::Gcc, true, true),
            component(Component::Gxx, false, false),
        ];
        assert_eq!(missing_packages(Tool::Cpp, &components), vec!["g++"]);
    }

    /// "Install all" must not re-fetch a compiler the machine already has.
    #[test]
    fn the_toolkit_installs_only_what_is_missing() {
        let components = vec![
            component(Component::Gcc, true, true),
            component(Component::Gxx, true, true),
            component(Component::Gfortran, false, false),
            component(Component::Make, true, true),
            component(Component::Cmake, false, false),
            component(Component::PkgConfig, true, true),
        ];
        assert_eq!(
            missing_packages(Tool::Toolkit, &components),
            vec!["gfortran", "cmake"]
        );
    }

    #[test]
    fn a_fully_installed_tool_needs_no_packages() {
        let components = vec![
            component(Component::Gcc, true, true),
            component(Component::Gxx, true, true),
        ];
        assert!(missing_packages(Tool::Cpp, &components).is_empty());
    }

    /// A component present but broken must still be reinstalled, which is what makes
    /// Repair different from doing nothing.
    #[test]
    fn a_broken_component_is_reinstalled_by_repair() {
        let components = vec![component(Component::Gfortran, true, false)];
        assert_eq!(
            missing_packages(Tool::Fortran, &components),
            vec!["gfortran"]
        );
    }

    /// Every package name the frontend can cause to be installed comes from this fixed set.
    #[test]
    fn the_package_allowlist_is_closed() {
        let allowed = ["gcc", "g++", "gfortran", "make", "cmake", "pkg-config"];
        for tool in [Tool::Cpp, Tool::Fortran, Tool::Build, Tool::Toolkit] {
            for component in tool.components() {
                assert!(
                    allowed.contains(&component.package()),
                    "{} is outside the allowlist",
                    component.package()
                );
            }
        }
    }

    // --- failure classification ------------------------------------------------------------

    #[test]
    fn a_held_package_lock_is_recognised() {
        assert_eq!(
            classify_apt_failure("E: Could not get lock /var/lib/dpkg/lock-frontend"),
            AptFailure::Busy
        );
    }

    #[test]
    fn a_network_failure_is_recognised() {
        assert_eq!(
            classify_apt_failure("Temporary failure resolving 'archive.ubuntu.com'"),
            AptFailure::Network
        );
        assert_eq!(
            classify_apt_failure("E: Failed to fetch http://archive.ubuntu.com/pool/main"),
            AptFailure::Network
        );
    }

    #[test]
    fn running_out_of_space_is_recognised() {
        assert_eq!(
            classify_apt_failure("dpkg: error: No space left on device"),
            AptFailure::Storage
        );
    }

    #[test]
    fn an_unrecognised_failure_falls_back_to_generic_advice() {
        assert_eq!(
            classify_apt_failure("something else went wrong"),
            AptFailure::Other
        );
    }

    /// Whatever the cause, the message must never suggest the user's work is at risk.
    #[test]
    fn every_failure_reassures_about_saved_work() {
        for failure in [
            AptFailure::Busy,
            AptFailure::Network,
            AptFailure::Storage,
            AptFailure::Other,
        ] {
            let error = apt_error(failure, "detail".into());
            let text = format!("{} {}", error.title, error.message).to_lowercase();
            assert!(
                text.contains("notebook") || text.contains("untouched"),
                "{:?} does not reassure: {}",
                failure,
                error.message
            );
        }
    }

    // --- report assembly ---------------------------------------------------------------------

    fn raw(works: &[(Component, bool)]) -> RawProbe {
        RawProbe {
            components: Tool::Toolkit
                .components()
                .iter()
                .map(|id| {
                    let ok = works
                        .iter()
                        .find(|(candidate, _)| candidate == id)
                        .is_some_and(|(_, ok)| *ok);
                    RawComponent {
                        id: *id,
                        present: ok,
                        works: ok,
                        version: ok.then(|| "13.2.0".to_string()),
                        detail: None,
                    }
                })
                .collect(),
            modules: [Tool::Seaborn, Tool::Statsmodels, Tool::Polars]
                .into_iter()
                .map(|id| RawModule {
                    id,
                    installed: true,
                    version: Some("0.13.2".into()),
                })
                .collect(),
        }
    }

    #[test]
    fn the_report_covers_every_tool_and_derives_the_toolkit() {
        let report = assemble(
            raw(&[
                (Component::Gcc, true),
                (Component::Gxx, true),
                (Component::Gfortran, false),
                (Component::Make, true),
                (Component::Cmake, true),
                (Component::PkgConfig, true),
            ]),
            "2026-09-18T00:00:00Z".into(),
        )
        .unwrap();

        let state = |id: Tool| {
            report
                .tools
                .iter()
                .find(|t| t.id == id)
                .map(|t| t.state.clone())
                .unwrap()
        };

        assert_eq!(state(Tool::Cpp), ToolState::Installed);
        assert_eq!(state(Tool::Build), ToolState::Installed);
        assert_eq!(state(Tool::Fortran), ToolState::NotInstalled);
        // One missing compiler makes the complete toolkit a repair, not a fresh install.
        assert_eq!(state(Tool::Toolkit), ToolState::NeedsRepair);
        assert_eq!(state(Tool::Seaborn), ToolState::Installed);
        assert_eq!(report.source, ReportSource::Verified);
    }

    #[test]
    fn partial_duplicate_and_contradictory_probes_are_rejected() {
        let mut partial = raw(&[(Component::Gcc, true)]);
        partial.components.retain(|c| c.id != Component::Gxx);
        assert!(assemble(partial, "t".into()).is_err());
        let mut duplicate = raw(&[]);
        duplicate.components[1].id = Component::Gcc;
        assert!(assemble(duplicate, "t".into()).is_err());
        let mut contradictory = raw(&[]);
        contradictory.components[0].works = true;
        assert!(assemble(contradictory, "t".into()).is_err());
        let mut wrong_module = raw(&[]);
        wrong_module.modules[0].id = Tool::Cpp;
        assert!(assemble(wrong_module, "t".into()).is_err());
    }

    #[test]
    fn a_verified_report_carries_versions_and_component_detail() {
        let report = assemble(
            raw(&[(Component::Gcc, true), (Component::Gxx, true)]),
            "t".into(),
        )
        .unwrap();
        let cpp = report.tools.iter().find(|t| t.id == Tool::Cpp).unwrap();
        assert_eq!(cpp.version.as_deref(), Some("13.2.0"));
        assert_eq!(cpp.components.len(), 2);
        assert!(cpp.components.iter().all(|c| !c.label.is_empty()));
    }
}
