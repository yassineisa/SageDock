//! The setup state machine that turns a bare Windows PC into a working SageMath
//! environment, installing from a local SageMath package rather than the internet.
//!
//! Stages are idempotent, so an interrupted setup (closed window, restart) is resumed by
//! simply running it again: completed work is detected and skipped. Every stage validates
//! its own result instead of trusting an exit code, and setup does not report success until
//! a real notebook cell has executed through the SageMath kernel.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult, ErrorSeverity};
use crate::runtime::{contract, image, workspace, wsl};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupStage {
    Preflight,
    InstallingWindowsComponents,
    WaitingForRestart,
    CheckingSagePackage,
    InstallingEnvironment,
    CreatingWorkspace,
    Verifying,
    Ready,
}

impl SetupStage {
    /// The user-facing stage name. Deliberately free of WSL/distro/archive vocabulary.
    pub fn title(self) -> &'static str {
        match self {
            SetupStage::Preflight => "Checking your PC",
            SetupStage::InstallingWindowsComponents => "Preparing Windows",
            SetupStage::WaitingForRestart => "Waiting for restart",
            SetupStage::CheckingSagePackage => "Checking the SageMath package",
            SetupStage::InstallingEnvironment => "Installing SageMath",
            SetupStage::CreatingWorkspace => "Creating your notebooks folder",
            SetupStage::Verifying => "Testing SageMath",
            SetupStage::Ready => "Ready",
        }
    }
}

/// What a progress report means, beyond which stage it belongs to.
///
/// The distinction exists because three things that look identical from outside — the app
/// working, the user not having answered a permission prompt, and Windows grinding away
/// invisibly — need completely different words on screen and completely different advice
/// when they run long.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressKind {
    /// SageDock is doing the work.
    Working,
    /// Nothing will happen until the user answers Windows' permission prompt.
    AwaitingPermission,
    /// Windows is applying changes SageDock cannot see inside.
    WaitingForWindows,
    /// Liveness only. Carries no new information and must never be shown as progress.
    Heartbeat,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupProgress {
    pub stage: SetupStage,
    pub title: String,
    pub detail: Option<String>,
    /// 0.0–1.0 within the current stage, when the stage can measure itself. `None` means
    /// the stage genuinely cannot measure itself — never a fabricated number to keep a bar
    /// moving.
    pub percent: Option<f32>,
    pub kind: ProgressKind,
}

impl SetupProgress {
    fn new(stage: SetupStage, detail: Option<&str>, percent: Option<f32>) -> Self {
        Self {
            stage,
            title: stage.title().to_string(),
            detail: detail.map(String::from),
            percent,
            kind: ProgressKind::Working,
        }
    }

    /// Waiting on somebody else. `on_user` distinguishes the prompt nobody has answered
    /// from Windows working by itself.
    fn waiting(stage: SetupStage, detail: &str, on_user: bool) -> Self {
        Self {
            stage,
            title: stage.title().to_string(),
            detail: Some(detail.to_string()),
            percent: None,
            kind: if on_user {
                ProgressKind::AwaitingPermission
            } else {
                ProgressKind::WaitingForWindows
            },
        }
    }

    /// Proof the supervising thread is alive, and nothing more.
    fn heartbeat(stage: SetupStage) -> Self {
        Self {
            stage,
            title: stage.title().to_string(),
            detail: None,
            percent: None,
            kind: ProgressKind::Heartbeat,
        }
    }
}

/// Persisted across restarts so setup can continue after a Windows reboot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedSetupState {
    pub awaiting_restart: bool,
    #[serde(default)]
    pub restart_requested_at: Option<u64>,
    pub runtime_version: Option<String>,
    pub completed: bool,
}

impl PersistedSetupState {
    /// A saved request describes a previous setup attempt, not an eternal Windows state.
    /// Older versions had no timestamp; their state-file modification time is the fallback.
    /// Unknown boot information retains the request rather than claiming a reboot happened.
    pub fn awaiting_restart_now(&self, dir: &Path) -> bool {
        if !self.awaiting_restart || self.completed {
            return false;
        }
        let legacy_time = std::fs::metadata(dir.join("setup-state.json"))
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        self.restart_pending_at(crate::system::boot_started_at(), legacy_time)
    }

    fn restart_pending_at(&self, boot: Option<u64>, legacy_time: Option<u64>) -> bool {
        self.awaiting_restart
            && !self.completed
            && !matches!(
                (boot, self.restart_requested_at.or(legacy_time)),
                (Some(boot), Some(requested)) if boot > requested
            )
    }

    fn request_restart(&mut self) {
        self.awaiting_restart = true;
        self.restart_requested_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs());
    }

    pub fn load(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join("setup-state.json"))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> AppResult<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| state_error(e.to_string()))?;
        crate::storage::atomic_write(&dir.join("setup-state.json"), &bytes)
            .map_err(|e| state_error(e.to_string()))
    }
}

fn state_error(details: String) -> AppError {
    AppError::new("setup", "SETUP_STATE_FAILED", "Setup couldn't finish safely",
        "Your notebooks are safe. SageDock couldn't save or verify its setup progress. Check available storage and try again.")
        .with_technical_details(details)
}

pub struct SetupPaths {
    pub app_data_dir: PathBuf,
    pub workspace_dir: PathBuf,
}

/// How setup finished. Only `Ready` means SageMath is usable; the others are normal
/// outcomes the UI presents as a next step, not as a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupOutcome {
    Ready,
    AwaitingRestart,
    /// SageMath isn't installed yet and no package file was available to install it from.
    NeedsSagePackage,
}

/// Runs setup as far as it can go, reporting progress along the way.
///
/// `sage_package` is only consulted if SageMath still needs installing, so re-running
/// setup on a machine where it is already installed never asks for the file again.
pub fn run_setup(
    paths: &SetupPaths,
    sage_package: Option<&Path>,
    operation_id: &str,
    on_progress: &mut dyn FnMut(SetupProgress),
) -> AppResult<SetupOutcome> {
    let mut state = PersistedSetupState::load(&paths.app_data_dir);
    on_progress(SetupProgress::new(SetupStage::Preflight, None, None));

    // A known same-boot request avoids repeated elevation. If boot information is
    // unavailable, Continue setup must still be able to validate the real components:
    // otherwise a blocked CIM service could trap the user in an endless restart prompt.
    if state.awaiting_restart
        && !state.completed
        && crate::system::boot_started_at().is_some()
        && state.awaiting_restart_now(&paths.app_data_dir)
    {
        on_progress(SetupProgress::new(
            SetupStage::WaitingForRestart,
            None,
            None,
        ));
        return Ok(SetupOutcome::AwaitingRestart);
    }

    // Whether this run is the continuation of one that already asked for a restart. Used
    // to make sure a second request can never be issued for the same cause, because a
    // restart that didn't help will never help however many times it is repeated.
    let resumed_after_restart = state.awaiting_restart;

    state.completed = false;
    state.save(&paths.app_data_dir)?;
    crate::runtime::health::preflight(
        &paths.app_data_dir,
        &paths.workspace_dir,
        !wsl::distro_exists(wsl::distro_name()),
    )?;

    // --- Windows components ----------------------------------------------------------
    // Both conditions are needed. `wsl --status` succeeding only proves `wsl.exe` runs;
    // a PC whose features are switched on but not yet active answers it happily and then
    // fails at the first operation needing a virtual machine. `vm_platform_present`
    // asks the question that actually matters.
    if !wsl::is_available() || !wsl::vm_platform_present() {
        on_progress(SetupProgress::new(
            SetupStage::InstallingWindowsComponents,
            Some("Windows will ask for permission to continue."),
            None,
        ));

        // The supervisor reports who it is waiting on, and beats while the helper works.
        // Both are forwarded so the UI can distinguish "answer the prompt" from "Windows is
        // busy" — the two situations that previously looked identical and together produced
        // the reported stall.
        let mut on_event = |event: wsl::ElevationEvent| {
            let stage = SetupStage::InstallingWindowsComponents;
            on_progress(match event {
                wsl::ElevationEvent::AwaitingConsent => SetupProgress::waiting(
                    stage,
                    "Windows is asking for permission. Look for the permission window — it \
                     can open behind SageDock or flash in the taskbar. Nothing continues \
                     until you answer it.",
                    true,
                ),
                wsl::ElevationEvent::HelperRunning => SetupProgress::waiting(
                    stage,
                    "Windows is switching on the components SageMath needs. This can take \
                     several minutes and often shows no activity while it works.",
                    false,
                ),
                wsl::ElevationEvent::Heartbeat => SetupProgress::heartbeat(stage),
            });
        };

        match wsl::install_wsl_elevated(&paths.app_data_dir, operation_id, &mut on_event)? {
            // Windows said it needs a reboot. This is the only path that may claim one:
            // it comes from Windows' own exit code, not from guessing after a failure.
            wsl::ElevatedOutcome::RestartRequired => {
                state.request_restart();
                state.save(&paths.app_data_dir)?;
                on_progress(SetupProgress::new(
                    SetupStage::WaitingForRestart,
                    None,
                    None,
                ));
                return Ok(SetupOutcome::AwaitingRestart);
            }
            wsl::ElevatedOutcome::Completed => {}
            wsl::ElevatedOutcome::DeclinedByUser => {
                return Err(AppError::new(
                    "setup",
                    "PERMISSION_DECLINED",
                    "SageDock needs your permission to continue",
                    "Windows asked for permission to install a component SageDock needs, and it wasn't granted. You can start setup again whenever you're ready.",
                )
                .with_severity(ErrorSeverity::Warning));
            }
        }

        // Enabling a Windows feature commonly succeeds "pending restart" without saying so
        // in its exit code, so the exit code alone cannot answer this.
        //
        // What settles it is whether the virtual machine platform is present *now*. That is
        // the component the first real operation needs, and its absence is exactly the state
        // that produced HCS_E_SERVICE_NOT_AVAILABLE on a test machine: Windows reported
        // success, setup continued, and `--import` then failed with exit code -1.
        //
        // Windows' registry restart flags are deliberately no longer consulted here. They
        // include `PendingFileRenameOperations`, which any installer can set for an unrelated
        // reason — an OEM updater had set it on the development machine, with 26 entries and
        // no Windows update pending at all — and trusting it sent healthy setups into a
        // restart they did not need. See `system::reboot` for the full reasoning.
        if !(wsl::is_available() && wsl::vm_platform_present()) && !resumed_after_restart {
            state.request_restart();
            state.save(&paths.app_data_dir)?;
            on_progress(SetupProgress::new(
                SetupStage::WaitingForRestart,
                None,
                None,
            ));
            return Ok(SetupOutcome::AwaitingRestart);
        }

        // Windows reported every step as succeeding outright, so WSL should answer now. If
        // it still doesn't, that is a real fault and is reported as one. The previous code
        // assumed a restart here, which turned any silent installation failure into an
        // endless reboot cycle: reboot, find WSL still missing, ask for another reboot.
        if !wsl::is_available() {
            return Err(AppError::new(
                "setup",
                "WSL_STILL_UNAVAILABLE",
                "Windows finished, but SageMath's environment still isn't available",
                "Windows reported that it switched on the components SageDock needs, but they still aren't responding. Restarting your computer once and running setup again usually resolves this.",
            ));
        }
    }

    state.awaiting_restart = false;
    state.restart_requested_at = None;
    state.save(&paths.app_data_dir)?;

    // --- SageMath environment --------------------------------------------------------
    if wsl::distro_exists(wsl::distro_name()) {
        read_runtime_info().map_err(|err| environment_needs_repair(err.technical_details))?;
    } else {
        let Some(package) = sage_package else {
            return Ok(SetupOutcome::NeedsSagePackage);
        };
        match install_from_package(paths, package, &mut state, on_progress) {
            Ok(()) => {}
            // The install reached Windows' virtual machine platform and found it dormant.
            // That is a restart, not a bad package — but only once: if a restart has
            // already been tried, the honest answer is the underlying failure.
            Err(err) if err.code == wsl::RESTART_REQUIRED_CODE && !resumed_after_restart => {
                state.request_restart();
                state.save(&paths.app_data_dir)?;
                on_progress(SetupProgress::new(
                    SetupStage::WaitingForRestart,
                    None,
                    None,
                ));
                return Ok(SetupOutcome::AwaitingRestart);
            }
            Err(err) => return Err(err),
        }
    }

    // --- workspace ---------------------------------------------------------------------
    on_progress(SetupProgress::new(
        SetupStage::CreatingWorkspace,
        None,
        None,
    ));
    workspace::ensure_workspace(&paths.workspace_dir)?;
    workspace::verify_writable(&paths.workspace_dir)?;

    // --- verify ------------------------------------------------------------------------
    on_progress(SetupProgress::new(
        SetupStage::Verifying,
        Some("Starting SageMath once to make sure everything works."),
        None,
    ));
    verify_installation()?;
    let mut server = crate::jupyter::start_ready(&paths.workspace_dir)?;
    let verified = crate::jupyter::verify_kernels(&server.session);
    server.stop();
    verified?;

    state.completed = true;
    state.save(&paths.app_data_dir)?;
    on_progress(SetupProgress::new(SetupStage::Ready, None, Some(1.0)));

    Ok(SetupOutcome::Ready)
}

fn install_from_package(
    paths: &SetupPaths,
    package: &Path,
    state: &mut PersistedSetupState,
    on_progress: &mut dyn FnMut(SetupProgress),
) -> AppResult<()> {
    const CHECKING: &str = "Making sure the file is complete.";

    on_progress(SetupProgress::new(
        SetupStage::CheckingSagePackage,
        Some(CHECKING),
        Some(0.0),
    ));
    let package = image::inspect(package)?;
    image::verify(&package, &mut |fraction| {
        on_progress(SetupProgress::new(
            SetupStage::CheckingSagePackage,
            Some(CHECKING),
            Some(fraction),
        ));
    })?;

    on_progress(SetupProgress::new(
        SetupStage::InstallingEnvironment,
        Some("This takes a few minutes."),
        None,
    ));
    crate::scientific::forget_cache(&paths.app_data_dir);
    wsl::import_distro(
        &wsl::default_install_dir(&paths.app_data_dir),
        &package.archive,
    )?;

    // A failure from here on is about the file, not the PC. The distro was created moments
    // ago by the import above, so removing it discards nothing except the bad import —
    // user files live on the Windows side and are never inside it.
    let info = match read_runtime_info() {
        Ok(info) => info,
        Err(err) => {
            tracing::warn!(target: "setup", code = %err.code, "imported file is not a usable SageDock image; removing it");
            let _ = wsl::unregister_distro();
            return Err(err);
        }
    };

    state.runtime_version = Some(info.sage_version);
    state.save(&paths.app_data_dir)?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct RuntimeInfo {
    format: u32,
    sage_version: String,
}

/// Reads the image's self-description and confirms its Jupyter launcher is present — the
/// minimum contract (see `contract`) that separates a SageDock image from an arbitrary
/// Linux archive.
fn read_runtime_info() -> AppResult<RuntimeInfo> {
    let info = wsl::run_as(wsl::LINUX_USER, &["cat", contract::RUNTIME_INFO])?;
    let launcher = wsl::run_as(wsl::LINUX_USER, &["test", "-x", contract::JUPYTER_LAUNCHER])?;
    parse_runtime_info(info.success, &info.combined_output, launcher.success)
}

fn parse_runtime_info(read_ok: bool, raw: &str, launcher_ok: bool) -> AppResult<RuntimeInfo> {
    if !read_ok || !launcher_ok {
        return Err(not_a_sagedock_image(tail(raw, 2000)));
    }

    // WSL can print its own warnings into the same output, so take the JSON object itself
    // rather than assuming the output is nothing but JSON.
    let json = match (raw.find('{'), raw.rfind('}')) {
        (Some(start), Some(end)) if end > start => &raw[start..=end],
        _ => return Err(not_a_sagedock_image(tail(raw, 2000))),
    };

    let info: RuntimeInfo =
        serde_json::from_str(json).map_err(|err| not_a_sagedock_image(err.to_string()))?;

    if info.format > image::SUPPORTED_FORMAT {
        return Err(image::needs_newer_app_error());
    }

    Ok(info)
}

/// Proves the environment works before setup may report success.
///
/// `factor(123456)` is the spec's suggested smoke test: it exercises real Sage evaluation
/// and has one unambiguous answer. The self-test then executes that same computation as a
/// notebook cell through the SageMath kernel, which is what a student will actually do.
pub fn verify_installation() -> AppResult<()> {
    let verify = wsl::run_as(wsl::LINUX_USER, &[contract::VERIFY])?;
    if !verify.success {
        return Err(state_error(
            "Runtime verification exited unsuccessfully".into(),
        ));
    }
    check_verify_output(&verify.combined_output)?;

    let selftest = wsl::run_as_timeout(wsl::LINUX_USER, &[contract::SELFTEST], 720)?;
    if !selftest.success {
        return Err(state_error(
            "Kernel verification exited unsuccessfully".into(),
        ));
    }
    check_selftest_output(&selftest.combined_output)?;

    let python = wsl::run_as_timeout(
        wsl::LINUX_USER,
        &[
            "/opt/sagedock/bin/sagedock-env",
            "python",
            "-c",
            r#"from jupyter_client import KernelManager
import time
km=KernelManager(kernel_name='python3'); km.start_kernel()
client=km.client(); client.start_channels()
try:
    client.wait_for_ready(timeout=60)
    request=client.execute('print(2 + 2)'); output=''; deadline=time.monotonic()+60
    while time.monotonic()<deadline:
        message=client.get_iopub_msg(timeout=60)
        if message.get('parent_header',{}).get('msg_id') != request: continue
        if message['msg_type']=='stream': output+=message['content']['text']
        if message['msg_type']=='error': raise RuntimeError('Python cell failed')
        if message['msg_type']=='status' and message['content']['execution_state']=='idle': break
    assert output.strip() == '4', 'Python kernel returned an unexpected result'
    print('PYTHON_KERNEL_OK')
finally:
    client.stop_channels(); km.shutdown_kernel(now=True)"#,
        ],
        150,
    )?;
    if !python.success
        || !python
            .combined_output
            .lines()
            .any(|s| s == "PYTHON_KERNEL_OK")
    {
        return Err(verify_error("PYTHON_KERNEL_TEST_FAILED", "Python notebooks couldn't run code",
            "Your notebooks are safe. Python's notebook engine did not pass its calculation test. Open Recovery and reinstall the computing environment if retrying doesn't help.", &python.combined_output));
    }

    tracing::info!(target: "setup", "installation verified");
    Ok(())
}

fn check_verify_output(output: &str) -> AppResult<()> {
    // 123456 = 2^6 * 3 * 643.
    if !output.contains("2^6 * 3 * 643") {
        return Err(verify_error(
            "SAGE_VERIFY_FAILED",
            "SageMath isn't responding correctly",
            "SageMath was installed, but it didn't produce the expected result when tested. Your notebooks won't be affected.",
            output,
        ));
    }

    if !output.contains("PYTHON_IMPORTS=ok") {
        return Err(verify_error(
            "PYTHON_VERIFY_FAILED",
            "Some of the scientific Python tools aren't working",
            "SageMath works, but some of the Python packages it comes with couldn't be loaded. Your notebooks won't be affected.",
            output,
        ));
    }

    // Checked on the KERNELS line specifically: the version line also says "SageMath", so a
    // search of the whole output would pass even with no kernel registered.
    let kernel_listed = output
        .lines()
        .find(|line| line.starts_with("KERNELS="))
        .is_some_and(|line| line.contains("sagemath"));

    if !kernel_listed {
        return Err(verify_error(
            "SAGE_KERNEL_MISSING",
            "SageDock couldn't find the SageMath notebook engine",
            "SageMath is installed, but the part that lets notebooks run Sage code wasn't found.",
            output,
        ));
    }

    Ok(())
}

fn check_selftest_output(output: &str) -> AppResult<()> {
    if output.contains("KERNEL_OK") {
        Ok(())
    } else {
        Err(verify_error(
            "SAGE_KERNEL_TEST_FAILED",
            "SageMath notebooks couldn't run code",
            "SageDock tried running a small calculation in a SageMath notebook and it didn't finish. Your notebooks won't be affected.",
            output,
        ))
    }
}

fn verify_error(code: &str, title: &str, message: &str, output: &str) -> AppError {
    AppError::new("setup", code, title, message).with_technical_details(tail(output, 4000))
}

fn not_a_sagedock_image(details: String) -> AppError {
    AppError::new(
        "setup",
        "SAGE_PACKAGE_NOT_SAGEDOCK",
        "That file isn't a SageMath package for SageDock",
        "The file installed, but it doesn't contain SageDock's SageMath setup, so SageDock removed it again. Nothing on your PC was changed. Choose the SageMath package that came with SageDock.",
    )
    .with_technical_details(details)
}

fn environment_needs_repair(details: Option<String>) -> AppError {
    let error = AppError::new(
        "setup",
        "ENVIRONMENT_INCOMPLETE",
        "SageDock's computing environment needs to be repaired",
        "Some internal files SageMath needs are missing or damaged. Your notebooks are stored separately and will not be affected.",
    );
    match details {
        Some(details) => error.with_technical_details(details),
        None => error,
    }
}

/// Keeps the last N bytes of long output (on a character boundary), since the useful part
/// of a failure is almost always at the end.
fn tail(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let start = text.len() - max;
    let boundary = (start..=text.len())
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(text.len());
    format!("…{}", &text[boundary..])
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_VERIFY: &str = "SAGE_VERSION=SageMath version 10.9, Release Date: 2026-05-20\n\
                               SAGE_FACTOR=2^6 * 3 * 643\n\
                               PYTHON_IMPORTS=ok\n\
                               KERNELS=Available kernels:   python3    /opt/x   sagemath   /opt/y ";

    #[test]
    fn stage_titles_avoid_technical_vocabulary() {
        let jargon = [
            "wsl", "distro", "apt", "kernel", "linux", "bash", "tar", "archive", "image",
        ];
        for stage in [
            SetupStage::Preflight,
            SetupStage::InstallingWindowsComponents,
            SetupStage::WaitingForRestart,
            SetupStage::CheckingSagePackage,
            SetupStage::InstallingEnvironment,
            SetupStage::CreatingWorkspace,
            SetupStage::Verifying,
            SetupStage::Ready,
        ] {
            let title = stage.title().to_lowercase();
            // Compared word by word, not as substrings: "restart" contains "tar".
            let words: Vec<&str> = title.split(|c: char| !c.is_alphanumeric()).collect();
            for word in jargon {
                assert!(
                    !words.contains(&word),
                    "stage title {title:?} leaks the word {word:?}"
                );
            }
        }
    }

    #[test]
    fn accepts_a_valid_runtime_description() {
        let info =
            parse_runtime_info(true, r#"{"format": 1, "sage_version": "10.9"}"#, true).unwrap();
        assert_eq!(info.sage_version, "10.9");
        assert_eq!(info.format, 1);
    }

    #[test]
    fn tolerates_wsl_warnings_around_the_json() {
        let raw = "wsl: A localhost proxy configuration was detected\n{\"format\":1,\"sage_version\":\"10.9\"}\n";
        assert!(parse_runtime_info(true, raw, true).is_ok());
    }

    /// A plain Ubuntu image imports fine but has no SageDock contract; it must be refused.
    #[test]
    fn rejects_an_image_without_the_sagedock_contract() {
        let missing_file =
            parse_runtime_info(false, "cat: /opt/sagedock/runtime.json: No such file", true);
        assert_eq!(missing_file.unwrap_err().code, "SAGE_PACKAGE_NOT_SAGEDOCK");

        let missing_launcher =
            parse_runtime_info(true, r#"{"format":1,"sage_version":"10.9"}"#, false);
        assert_eq!(
            missing_launcher.unwrap_err().code,
            "SAGE_PACKAGE_NOT_SAGEDOCK"
        );

        let garbage = parse_runtime_info(true, "not json at all", true);
        assert_eq!(garbage.unwrap_err().code, "SAGE_PACKAGE_NOT_SAGEDOCK");
    }

    #[test]
    fn refuses_an_image_from_a_newer_app_version() {
        let raw = format!(
            r#"{{"format": {}, "sage_version": "11.0"}}"#,
            image::SUPPORTED_FORMAT + 1
        );
        assert_eq!(
            parse_runtime_info(true, &raw, true).unwrap_err().code,
            "SAGE_PACKAGE_TOO_NEW"
        );
    }

    #[test]
    fn healthy_verify_output_passes() {
        assert!(check_verify_output(GOOD_VERIFY).is_ok());
    }

    #[test]
    fn wrong_factor_result_fails() {
        let output =
            GOOD_VERIFY.replace("2^6 * 3 * 643", "NameError: name 'factor' is not defined");
        assert_eq!(
            check_verify_output(&output).unwrap_err().code,
            "SAGE_VERIFY_FAILED"
        );
    }

    #[test]
    fn broken_python_packages_fail() {
        let output = GOOD_VERIFY.replace(
            "PYTHON_IMPORTS=ok",
            "PYTHON_IMPORTS=ModuleNotFoundError: pandas",
        );
        assert_eq!(
            check_verify_output(&output).unwrap_err().code,
            "PYTHON_VERIFY_FAILED"
        );
    }

    /// Regression guard: "SageMath" appears in the version line, so a naive whole-output
    /// search would report a kernel that doesn't exist.
    #[test]
    fn kernel_must_be_on_the_kernels_line() {
        let output = GOOD_VERIFY.replace("sagemath   /opt/y", "");
        assert!(output.contains("SageMath version"));
        assert_eq!(
            check_verify_output(&output).unwrap_err().code,
            "SAGE_KERNEL_MISSING"
        );
    }

    #[test]
    fn selftest_requires_the_ok_marker() {
        assert!(check_selftest_output("KERNEL_OK\n").is_ok());
        assert_eq!(
            check_selftest_output("KERNEL_FAIL\nTimeout waiting for kernel")
                .unwrap_err()
                .code,
            "SAGE_KERNEL_TEST_FAILED"
        );
    }

    #[test]
    fn tail_keeps_the_end_of_long_output() {
        let text = "a".repeat(100) + "IMPORTANT";
        let out = tail(&text, 20);
        assert!(out.ends_with("IMPORTANT"));
    }

    #[test]
    fn tail_handles_multibyte_characters_without_panicking() {
        let _ = tail(&"é".repeat(500), 101);
    }

    #[test]
    fn setup_state_round_trips() {
        let dir = std::env::temp_dir().join(format!("sagedock-setup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        PersistedSetupState {
            awaiting_restart: true,
            restart_requested_at: Some(100),
            runtime_version: Some("10.9".into()),
            completed: false,
        }
        .save(&dir)
        .unwrap();

        let loaded = PersistedSetupState::load(&dir);
        assert!(loaded.awaiting_restart);
        assert_eq!(loaded.restart_requested_at, Some(100));
        assert_eq!(loaded.runtime_version.as_deref(), Some("10.9"));
    }

    #[test]
    fn missing_state_file_falls_back_to_defaults() {
        let dir =
            std::env::temp_dir().join(format!("sagedock-setup-missing-{}", std::process::id()));
        let loaded = PersistedSetupState::load(&dir);
        assert!(!loaded.awaiting_restart && !loaded.completed);
    }

    #[test]
    fn restart_request_survives_retry_but_clears_after_a_new_boot() {
        let state = PersistedSetupState {
            awaiting_restart: true,
            restart_requested_at: Some(200),
            ..Default::default()
        };
        assert!(state.restart_pending_at(Some(100), None));
        assert!(!state.restart_pending_at(Some(300), None));
        assert!(
            state.restart_pending_at(None, None),
            "unknown is not proof of a restart"
        );
    }

    #[test]
    fn legacy_restart_requests_use_the_saved_file_time() {
        let state: PersistedSetupState = serde_json::from_str(
            r#"{"awaiting_restart":true,"completed":false,"runtime_version":null}"#,
        )
        .unwrap();
        assert!(state.restart_pending_at(Some(100), Some(200)));
        assert!(!state.restart_pending_at(Some(300), Some(200)));
        assert!(state.restart_pending_at(Some(300), None));
    }

    #[test]
    fn completed_setup_cannot_keep_a_stale_restart_warning() {
        let state = PersistedSetupState {
            awaiting_restart: true,
            completed: true,
            ..Default::default()
        };
        assert!(!state.restart_pending_at(None, None));
    }
}
