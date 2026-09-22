//! Backend ownership of the setup operation.
//!
//! Setup used to be owned by the screen that started it: Home called `run_setup`, awaited
//! the promise, and displayed "Setting up SageMath" for exactly as long as that promise was
//! pending. Everything about that arrangement was fragile. Navigating away unsubscribed the
//! only listener for `setup-progress`, so events emitted while the user was on another page
//! were lost with nothing to recover them from; and because the progress card required both
//! a live event *and* the frontend busy flag, returning to Home showed a disabled screen
//! with no explanation. A stalled command left the app permanently "busy" with no way back.
//!
//! Here the operation belongs to the backend. It runs on its own thread, publishes an
//! authoritative [`SetupSnapshot`] that any screen can query at any time, and emits that
//! same snapshot as an event for live updates. Screens become pure observers: they can
//! mount, unmount, and remount freely, and they always see the truth by asking for it.
//!
//! Two invariants make reconnection safe:
//!
//! - **Every snapshot carries `operation_id` and `seq`.** A screen ignores any snapshot
//!   whose `seq` is not newer than the one it already has for that operation, so a slow
//!   query answering after a fast event cannot roll the display backwards.
//! - **Every operation reaches a recorded terminal phase.** The phase is persisted, so a
//!   crash or a Windows restart leaves evidence rather than ambiguity, and the next launch
//!   reconciles it against what is actually installed instead of trusting the stored flag.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::runtime::provision::{SetupOutcome, SetupStage};

/// Where the last operation's outcome is recorded, for reconciliation after a crash.
const OPERATION_FILE: &str = "setup-operation.json";

/// How many log lines the technical view retains. Enough to cover a whole setup run
/// without letting a chatty stage grow the snapshot without bound, the snapshot is cloned
/// on every query and serialised on every event.
const MAX_LOG_LINES: usize = 500;

/// Epoch milliseconds. Used rather than an instant because the UI renders absolute times
/// and a persisted operation has to stay comparable across a restart.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// What setup is doing, at the level a student is asked to act on.
///
/// This is deliberately coarser than [`SetupStage`]: a stage says which step is running,
/// a phase says whether anybody needs to do anything about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupPhase {
    /// No setup has run in this session and none was left unfinished.
    Idle,
    /// SageDock is working. The app is the active party.
    Running,
    /// Windows is showing its permission prompt. The *user* is the active party, and no
    /// progress can happen until they answer.
    WaitingForPermission,
    /// Windows is applying changes that SageDock cannot observe from outside. Waiting is
    /// correct; this is not a stall.
    WaitingForWindows,
    /// Windows needs a restart before setup can continue.
    RestartRequired,
    /// Setup finished and the runtime was verified end to end.
    Completed,
    /// Setup stopped on an error. `problem` explains it and what to do.
    Failed,
    /// A previous run never recorded an ending, the app closed, crashed, or Windows
    /// restarted mid-operation. Needs reconciling against what is actually installed
    /// before anything is retried.
    Interrupted,
}

impl SetupPhase {
    /// Whether this phase means work is still outstanding in some form.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            SetupPhase::Completed
                | SetupPhase::Failed
                | SetupPhase::RestartRequired
                | SetupPhase::Interrupted
        )
    }

    /// Whether a background thread should still be running for this phase. Used by
    /// reconciliation to decide whether a persisted record describes a live operation.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            SetupPhase::Running | SetupPhase::WaitingForPermission | SetupPhase::WaitingForWindows
        )
    }
}

/// How one step of the plan is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Pending,
    Active,
    Done,
    /// Not needed on this PC, the usual case being Windows components that are already on.
    Skipped,
    Failed,
}

/// One step as the UI lists it: what it is, why it exists, and where it has got to.
#[derive(Debug, Clone, Serialize)]
pub struct SetupStep {
    pub stage: SetupStage,
    pub title: String,
    /// Why this step exists, in plain language. Shown so the user can tell whether a slow
    /// step is doing something that warrants the wait.
    pub explanation: String,
    pub state: StepState,
}

/// The ordered plan. `WaitingForRestart` and `Ready` are excluded deliberately: the first
/// is a branch out of the plan rather than a step in it, and the second is the plan being
/// finished, not another thing to do.
pub fn plan_stages() -> &'static [SetupStage] {
    &[
        SetupStage::Preflight,
        SetupStage::InstallingWindowsComponents,
        SetupStage::InstallingEnvironment,
        SetupStage::CreatingWorkspace,
        SetupStage::Verifying,
    ]
}

/// Why each step exists. Kept beside the plan so a new stage cannot be added to the list
/// without someone having to write the sentence that explains it to a student.
pub fn explain_stage(stage: SetupStage) -> &'static str {
    match stage {
        SetupStage::Preflight => {
            "Checks this PC can run SageMath before anything is changed: Windows version, \
             virtualisation, free space, and whether the SageMath package is readable."
        }
        SetupStage::InstallingWindowsComponents => {
            "Switches on the Windows features SageMath runs inside. This is the only step \
             that needs administrator permission, and Windows may ask for a restart."
        }
        SetupStage::WaitingForRestart => {
            "Windows has to restart to finish switching on the components it just enabled. \
             Your progress is saved and setup continues afterwards."
        }
        SetupStage::CheckingSagePackage => {
            "Confirms the SageMath package that shipped with SageDock is complete and \
             undamaged before it is installed."
        }
        SetupStage::InstallingEnvironment => {
            "Unpacks SageMath, Python, and Jupyter onto this PC. This is the longest step \
             and it does not need the internet. Everything installs from the local package."
        }
        SetupStage::CreatingWorkspace => {
            "Creates your notebooks folder in Windows. Your work is stored here, outside \
             the SageMath environment, so repairing or reinstalling never touches it."
        }
        SetupStage::Verifying => {
            "Runs SageMath and Python for real and executes a test notebook cell in each. \
             Setup only reports success once both kernels have actually answered."
        }
        SetupStage::Ready => "Setup is finished and SageMath is ready to use.",
    }
}

/// A line for the expandable technical view. Free of paths under the user's profile and of
/// anything resembling a token, see `redact` below.
#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub at: u64,
    pub text: String,
}

/// The authoritative picture of the setup operation. Every field the UI renders comes from
/// here, so there is exactly one description of what is happening.
///
/// Serialize-only on purpose. What survives a crash is [`PersistedOperation`], a much
/// smaller record; the rest of the snapshot is rebuilt from the machine's actual state on
/// the next launch rather than deserialised from a file a dead process wrote.
#[derive(Debug, Clone, Serialize)]
pub struct SetupSnapshot {
    /// Identifies one run. A snapshot from a previous run must never overwrite a newer
    /// one, and the id is what lets a reconnecting screen tell the difference.
    pub operation_id: String,
    /// Increases on every published change. Monotonic within an operation.
    pub seq: u64,
    pub phase: SetupPhase,
    pub stage: Option<SetupStage>,
    /// Plain-language name of what is happening now.
    pub title: String,
    pub detail: Option<String>,
    /// Only ever set from something actually measured. Never synthesised to make a bar
    /// move: an unmeasurable stage reports `None` and the UI shows indeterminate progress.
    pub percent: Option<f32>,
    pub steps: Vec<SetupStep>,
    pub started_at: u64,
    /// When the stage or phase last genuinely changed. This is what "last progress" means
    /// in the UI, and it is deliberately not touched by the heartbeat.
    pub updated_at: u64,
    /// Proof the operation thread is alive. Explicitly *not* evidence of progress, and the
    /// UI is required to label it that way.
    pub heartbeat_at: u64,
    pub outcome: Option<SetupOutcome>,
    pub problem: Option<AppError>,
    pub log: Vec<LogLine>,
}

impl SetupSnapshot {
    fn idle() -> Self {
        Self {
            operation_id: String::new(),
            seq: 0,
            phase: SetupPhase::Idle,
            stage: None,
            title: "Setup hasn't started".into(),
            detail: None,
            percent: None,
            steps: initial_steps(),
            started_at: 0,
            updated_at: 0,
            heartbeat_at: 0,
            outcome: None,
            problem: None,
            log: Vec::new(),
        }
    }

    /// How long the UI should say this has been going, in milliseconds. `None` once the
    /// operation has ended, because a finished run has a duration, not an elapsed time.
    pub fn elapsed_ms(&self, now: u64) -> Option<u64> {
        if self.started_at == 0 || self.phase.is_terminal() {
            return None;
        }
        Some(now.saturating_sub(self.started_at))
    }
}

fn initial_steps() -> Vec<SetupStep> {
    plan_stages()
        .iter()
        .map(|&stage| SetupStep {
            stage,
            title: stage.title().to_string(),
            explanation: explain_stage(stage).to_string(),
            state: StepState::Pending,
        })
        .collect()
}

/// The record persisted between runs. Deliberately a subset of the snapshot: the log and
/// the step list are rebuilt rather than trusted, because what matters after a crash is
/// *what actually happened on disk*, not what the previous process believed.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedOperation {
    operation_id: String,
    phase: SetupPhase,
    stage: Option<SetupStage>,
    started_at: u64,
    updated_at: u64,
    /// The process that owned the operation. Used to tell "still running" from "died
    /// without recording an ending" without trusting the phase alone.
    pid: u32,
}

/// Publishes snapshots to whoever is listening. Implemented by the Tauri app handle in
/// `commands`, and by a recording stub in tests, so the state machine itself can be tested
/// without an event loop.
pub trait ProgressSink: Send + Sync {
    fn publish(&self, snapshot: &SetupSnapshot);
}

/// Owns the current snapshot and the sequence counter.
///
/// Held in `AppState`. Mutation always goes through [`Self::update`], which is what keeps
/// `seq` monotonic and the published copy identical to the queryable one.
pub struct SetupTracker {
    current: Mutex<SetupSnapshot>,
    seq: AtomicU64,
    app_data_dir: PathBuf,
}

impl SetupTracker {
    pub fn new(app_data_dir: PathBuf) -> Self {
        Self {
            current: Mutex::new(SetupSnapshot::idle()),
            seq: AtomicU64::new(0),
            app_data_dir,
        }
    }

    /// The current picture. Always answerable, including before any setup has run.
    pub fn snapshot(&self) -> SetupSnapshot {
        self.current.lock().expect("setup mutex poisoned").clone()
    }

    /// Applies a change, bumps `seq`, and publishes. The sole mutation path.
    fn update(&self, sink: Option<&dyn ProgressSink>, mutate: impl FnOnce(&mut SetupSnapshot)) {
        let published = {
            let mut guard = self.current.lock().expect("setup mutex poisoned");
            mutate(&mut guard);
            guard.seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
            guard.clone()
        };
        if let Some(sink) = sink {
            sink.publish(&published);
        }
    }

    /// Starts a new operation, returning its id.
    pub fn begin(&self, sink: Option<&dyn ProgressSink>) -> String {
        let id = new_operation_id();
        let at = now_ms();
        self.update(sink, |s| {
            *s = SetupSnapshot {
                operation_id: id.clone(),
                seq: s.seq,
                phase: SetupPhase::Running,
                stage: Some(SetupStage::Preflight),
                title: SetupStage::Preflight.title().to_string(),
                detail: None,
                percent: None,
                steps: initial_steps(),
                started_at: at,
                updated_at: at,
                heartbeat_at: at,
                outcome: None,
                problem: None,
                log: Vec::new(),
            };
        });
        self.persist(SetupPhase::Running, Some(SetupStage::Preflight), &id, at);
        id
    }

    /// Moves to a stage, marking earlier plan steps done and this one active.
    pub fn enter_stage(
        &self,
        sink: Option<&dyn ProgressSink>,
        stage: SetupStage,
        detail: Option<&str>,
        percent: Option<f32>,
    ) {
        let at = now_ms();
        let mut id = String::new();
        self.update(sink, |s| {
            s.stage = Some(stage);
            s.title = stage.title().to_string();
            s.detail = detail.map(str::to_string);
            s.percent = percent;
            s.updated_at = at;
            s.heartbeat_at = at;
            // A stage that is not in the plan (a restart branch) must not reorder the
            // steps: only advance the list when the stage actually appears in it.
            if let Some(index) = s.steps.iter().position(|step| step.stage == stage) {
                for (i, step) in s.steps.iter_mut().enumerate() {
                    match (i.cmp(&index), step.state) {
                        // Ran and finished.
                        (std::cmp::Ordering::Less, StepState::Active) => {
                            step.state = StepState::Done
                        }
                        // Passed over without ever being entered: this PC didn't need it.
                        (std::cmp::Ordering::Less, StepState::Pending) => {
                            step.state = StepState::Skipped
                        }
                        (std::cmp::Ordering::Equal, _) => step.state = StepState::Active,
                        _ => {}
                    }
                }
            }
            id = s.operation_id.clone();
        });
        self.append_log(sink, &format!("Stage: {}", stage.title()));
        self.persist(SetupPhase::Running, Some(stage), &id, at);
    }

    /// Switches the phase without changing the stage, used when the app starts waiting on
    /// somebody else, so the UI can say who.
    pub fn set_phase(&self, sink: Option<&dyn ProgressSink>, phase: SetupPhase, detail: &str) {
        let at = now_ms();
        let mut id = String::new();
        let mut stage = None;
        self.update(sink, |s| {
            s.phase = phase;
            s.detail = Some(detail.to_string());
            s.updated_at = at;
            s.heartbeat_at = at;
            id = s.operation_id.clone();
            stage = s.stage;
        });
        self.append_log(sink, detail);
        self.persist(phase, stage, &id, at);
    }

    /// Liveness only.
    ///
    /// This deliberately does not touch `updated_at`. A heartbeat proves the supervising
    /// thread is alive; it says nothing about whether the installation is advancing, and
    /// presenting it as progress is exactly the false reassurance this module exists to
    /// avoid. The UI shows elapsed time since `updated_at`, which keeps growing during a
    /// silent stage and is what lets it offer a next step when a stage runs long.
    pub fn heartbeat(&self, sink: Option<&dyn ProgressSink>) {
        self.update(sink, |s| s.heartbeat_at = now_ms());
    }

    /// Adds a line to the technical view, redacted and capped.
    pub fn append_log(&self, sink: Option<&dyn ProgressSink>, text: &str) {
        let line = LogLine {
            at: now_ms(),
            text: redact(text),
        };
        self.update(sink, |s| {
            s.log.push(line);
            if s.log.len() > MAX_LOG_LINES {
                let excess = s.log.len() - MAX_LOG_LINES;
                s.log.drain(..excess);
            }
        });
    }

    /// Ends the operation in a recorded terminal phase.
    pub fn finish(
        &self,
        sink: Option<&dyn ProgressSink>,
        phase: SetupPhase,
        outcome: Option<SetupOutcome>,
        problem: Option<AppError>,
    ) {
        let at = now_ms();
        let mut id = String::new();
        self.update(sink, |s| {
            s.phase = phase;
            s.outcome = outcome;
            s.updated_at = at;
            s.heartbeat_at = at;
            s.percent = None;
            match phase {
                SetupPhase::Completed => {
                    s.stage = Some(SetupStage::Ready);
                    s.title = SetupStage::Ready.title().to_string();
                    s.detail = Some("SageMath and Python both ran a test notebook cell.".into());
                    for step in &mut s.steps {
                        if step.state == StepState::Active || step.state == StepState::Pending {
                            step.state = StepState::Done;
                        }
                    }
                }
                SetupPhase::RestartRequired => {
                    s.stage = Some(SetupStage::WaitingForRestart);
                    s.title = SetupStage::WaitingForRestart.title().to_string();
                }
                SetupPhase::Failed => {
                    if let Some(active) = s.steps.iter_mut().find(|x| x.state == StepState::Active)
                    {
                        active.state = StepState::Failed;
                    }
                    if let Some(err) = &problem {
                        s.title = err.title.clone();
                        s.detail = Some(err.message.clone());
                    }
                }
                _ => {}
            }
            s.problem = problem.clone();
            id = s.operation_id.clone();
        });
        self.persist(phase, None, &id, at);
    }

    // --- persistence and reconciliation ---------------------------------------------------

    /// Writes the operation record. Failures are logged, never surfaced: being unable to
    /// write the reconciliation file must not stop an installation that is otherwise fine.
    /// The cost is that a crash then looks like a clean slate, which is the safe direction.
    fn persist(&self, phase: SetupPhase, stage: Option<SetupStage>, id: &str, at: u64) {
        if id.is_empty() {
            return;
        }
        let record = PersistedOperation {
            operation_id: id.to_string(),
            phase,
            stage,
            started_at: self
                .current
                .lock()
                .map(|s| s.started_at)
                .unwrap_or_default(),
            updated_at: at,
            pid: std::process::id(),
        };
        let Ok(bytes) = serde_json::to_vec_pretty(&record) else {
            return;
        };
        if let Err(err) =
            crate::storage::atomic_write(&self.app_data_dir.join(OPERATION_FILE), &bytes)
        {
            tracing::warn!(target: "setup", error = %err, "could not record setup progress");
        }
    }

    /// Reads a previous run's record on launch and decides what it means now.
    ///
    /// The stored phase is never trusted on its own. A record saying "running" written by a
    /// process that no longer exists describes a crash, not an installation in flight, and
    /// restoring it as running would leave the app busy forever, the exact failure this
    /// module was written to remove. `installed_now` supplies the ground truth the caller
    /// has already established by looking at the machine.
    pub fn reconcile(&self, sink: Option<&dyn ProgressSink>, installed_now: bool) {
        let Some(record) = self.read_record() else {
            return;
        };
        // Our own pid means this file belongs to the operation we are already tracking.
        if record.pid == std::process::id() {
            return;
        }
        if !record.phase.is_active() {
            return;
        }
        let at = now_ms();
        self.update(sink, |s| {
            s.operation_id = record.operation_id.clone();
            s.started_at = record.started_at;
            s.updated_at = at;
            s.heartbeat_at = at;
            s.steps = initial_steps();
            if installed_now {
                // The work survived the interruption. Say so plainly rather than inviting a
                // reinstall of something that is already there.
                s.phase = SetupPhase::Interrupted;
                s.title = "Setup was interrupted".into();
                s.detail = Some(
                    "SageDock closed while it was setting up. SageMath is installed, so \
                     choosing Continue setup picks up where it left off. Nothing is \
                     reinstalled and your files were not affected."
                        .into(),
                );
            } else {
                s.phase = SetupPhase::Interrupted;
                s.title = "Setup was interrupted".into();
                s.detail = Some(
                    "SageDock closed before SageMath finished installing. Choosing Continue \
                     setup starts again from the last completed step. Your files were not \
                     affected."
                        .into(),
                );
            }
        });
    }

    fn read_record(&self) -> Option<PersistedOperation> {
        let raw = std::fs::read_to_string(self.app_data_dir.join(OPERATION_FILE)).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// Clears the record once its outcome has been absorbed, so a reconciled interruption
    /// is not reported a second time on the next launch.
    pub fn clear_record(&self) -> AppResult<()> {
        let path = self.app_data_dir.join(OPERATION_FILE);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(AppError::new(
                "setup",
                "SETUP_RECORD_CLEAR_FAILED",
                "SageDock couldn't tidy up its setup record",
                "Your files are safe. SageDock could not remove the file it uses to track \
                 an interrupted setup, so it may mention the interruption again.",
            )
            .with_technical_details(err.to_string())),
        }
    }
}

/// An operation identifier. Random enough to distinguish runs within a session and across
/// restarts, which is all it is relied upon for, it is not a security token, and nothing
/// grants access on the strength of holding one.
fn new_operation_id() -> String {
    use std::hash::{BuildHasher, Hasher, RandomState};
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(now_ms());
    hasher.write_u32(std::process::id());
    format!("{:016x}", hasher.finish())
}

/// Strips the two things that would otherwise make an exported diagnostic unsafe to share:
/// the user's profile path (which carries their Windows account name) and anything shaped
/// like a token or key.
///
/// This is a deliberately blunt filter over text SageDock itself writes. It is not a
/// general-purpose sanitiser for arbitrary third-party output, and it is not relied on to
/// make untrusted content safe, it reduces avoidable personal detail in a file the user is
/// invited to send to somebody else.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&redact_line(line));
    }
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn redact_line(line: &str) -> String {
    let lowered = line.to_ascii_lowercase();
    // A line that announces a secret is dropped wholesale rather than partially masked:
    // guessing where the value starts is how half a token ends up in a log.
    for marker in [
        "token=", "token:", "password", "secret", "api_key", "apikey",
    ] {
        if lowered.contains(marker) {
            return "[redacted]".to_string();
        }
    }
    redact_user_path(line)
}

/// Replaces `C:\Users\<name>` with a placeholder, keeping the rest of the path readable.
fn redact_user_path(line: &str) -> String {
    let lowered = line.to_ascii_lowercase();
    let mut result = String::new();
    let mut rest = line;
    let mut lowered_rest = lowered.as_str();
    while let Some(index) = lowered_rest.find("\\users\\") {
        let (before, after) = rest.split_at(index);
        result.push_str(before);
        result.push_str("\\Users\\<user>");
        // Skip past "\users\" and the account name that follows it.
        let tail = &after["\\users\\".len()..];
        let cut = tail.find(['\\', '/', '"', ' ']).unwrap_or(tail.len());
        rest = &tail[cut..];
        lowered_rest = &lowered_rest[index + "\\users\\".len() + cut..];
    }
    result.push_str(rest);
    result
}

/// Whether a process with this id is currently running.
///
/// Used to tell a crashed operation from a live one, and, more importantly during
/// elevation, to tell a genuinely hung helper from one that is simply quiet. Quiet output
/// is not evidence of a hang: `dism` can spend many minutes saying nothing at all.
#[cfg(windows)]
pub fn process_is_running(pid: u32) -> bool {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // `tasklist` filtered by pid prints a header and the row only when the process exists.
    let Ok(output) = Command::new("tasklist.exe")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .output()
    else {
        // Unable to ask is not the same as "gone". Reporting it as running keeps the
        // supervisor patient rather than declaring a false timeout on a live install.
        return true;
    };
    String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
}

#[cfg(not(windows))]
pub fn process_is_running(_pid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Records everything published, so ordering and sequence can be asserted without an
    /// event loop.
    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<SetupSnapshot>>,
    }

    impl ProgressSink for Recorder {
        fn publish(&self, snapshot: &SetupSnapshot) {
            self.seen.lock().unwrap().push(snapshot.clone());
        }
    }

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::AtomicU32;
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("sagedock-setup-test-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tracker() -> (SetupTracker, Arc<Recorder>) {
        (SetupTracker::new(temp_dir()), Arc::new(Recorder::default()))
    }

    #[test]
    fn an_untouched_tracker_still_answers_with_an_idle_snapshot() {
        let (tracker, _) = tracker();
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.phase, SetupPhase::Idle);
        assert_eq!(snapshot.seq, 0);
        // The plan is listed before anything runs, so the explanation panel can show what
        // setup will do without having to start it first.
        assert_eq!(snapshot.steps.len(), plan_stages().len());
        assert!(snapshot.steps.iter().all(|s| s.state == StepState::Pending));
    }

    #[test]
    fn every_published_update_carries_a_higher_sequence_than_the_last() {
        let (tracker, sink) = tracker();
        let listener: &dyn ProgressSink = sink.as_ref();
        tracker.begin(Some(listener));
        tracker.enter_stage(
            Some(listener),
            SetupStage::InstallingEnvironment,
            None,
            None,
        );
        tracker.heartbeat(Some(listener));
        tracker.finish(
            Some(listener),
            SetupPhase::Completed,
            Some(SetupOutcome::Ready),
            None,
        );

        let seen = sink.seen.lock().unwrap();
        assert!(
            seen.len() >= 4,
            "expected several updates, saw {}",
            seen.len()
        );
        for pair in seen.windows(2) {
            assert!(
                pair[1].seq > pair[0].seq,
                "sequence went backwards: {} then {}",
                pair[0].seq,
                pair[1].seq,
            );
        }
    }

    #[test]
    fn a_new_run_gets_an_id_that_differs_from_the_one_before_it() {
        let (tracker, _) = tracker();
        let first = tracker.begin(None);
        // Same millisecond is the hard case: ids must still differ, or a fast retry would
        // look to the UI like the original operation resurrecting.
        let second = tracker.begin(None);
        assert_ne!(first, second);
    }

    #[test]
    fn a_heartbeat_moves_liveness_but_never_the_progress_timestamp() {
        let (tracker, _) = tracker();
        tracker.begin(None);
        tracker.enter_stage(None, SetupStage::InstallingEnvironment, None, None);
        let before = tracker.snapshot();
        std::thread::sleep(std::time::Duration::from_millis(5));
        tracker.heartbeat(None);
        let after = tracker.snapshot();
        assert_eq!(
            before.updated_at, after.updated_at,
            "a heartbeat must not be recorded as progress",
        );
        assert!(after.heartbeat_at >= before.heartbeat_at);
    }

    #[test]
    fn entering_a_later_stage_marks_the_skipped_ones_rather_than_leaving_them_pending() {
        let (tracker, _) = tracker();
        tracker.begin(None);
        // A PC whose Windows components are already on goes straight to installing.
        tracker.enter_stage(None, SetupStage::InstallingEnvironment, None, None);
        let snapshot = tracker.snapshot();
        let windows = snapshot
            .steps
            .iter()
            .find(|s| s.stage == SetupStage::InstallingWindowsComponents)
            .expect("windows step");
        assert_eq!(windows.state, StepState::Skipped);
        let installing = snapshot
            .steps
            .iter()
            .find(|s| s.stage == SetupStage::InstallingEnvironment)
            .expect("install step");
        assert_eq!(installing.state, StepState::Active);
    }

    #[test]
    fn a_restart_branch_does_not_reorder_the_plan() {
        let (tracker, _) = tracker();
        tracker.begin(None);
        tracker.enter_stage(None, SetupStage::InstallingWindowsComponents, None, None);
        // WaitingForRestart is not in the plan; entering it must leave the list alone.
        tracker.enter_stage(None, SetupStage::WaitingForRestart, None, None);
        let snapshot = tracker.snapshot();
        assert!(
            snapshot
                .steps
                .iter()
                .all(|s| s.stage != SetupStage::WaitingForRestart),
            "the restart branch must not appear as a plan step",
        );
        let installing = snapshot
            .steps
            .iter()
            .find(|s| s.stage == SetupStage::InstallingEnvironment)
            .expect("install step");
        assert_eq!(
            installing.state,
            StepState::Pending,
            "a restart must not mark later steps as skipped",
        );
    }

    #[test]
    fn a_failure_marks_the_running_step_failed_and_keeps_the_reason() {
        let (tracker, _) = tracker();
        tracker.begin(None);
        tracker.enter_stage(None, SetupStage::Verifying, None, None);
        let problem = AppError::new(
            "setup",
            "KERNEL_FAILED",
            "SageMath didn't answer",
            "Try again.",
        );
        tracker.finish(None, SetupPhase::Failed, None, Some(problem));

        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.phase, SetupPhase::Failed);
        let verifying = snapshot
            .steps
            .iter()
            .find(|s| s.stage == SetupStage::Verifying)
            .expect("verify step");
        assert_eq!(verifying.state, StepState::Failed);
        assert_eq!(
            snapshot.problem.expect("problem kept").code,
            "KERNEL_FAILED"
        );
    }

    #[test]
    fn completion_leaves_no_step_unaccounted_for() {
        let (tracker, _) = tracker();
        tracker.begin(None);
        tracker.enter_stage(None, SetupStage::Verifying, None, None);
        tracker.finish(None, SetupPhase::Completed, Some(SetupOutcome::Ready), None);
        let snapshot = tracker.snapshot();
        assert!(
            snapshot
                .steps
                .iter()
                .all(|s| matches!(s.state, StepState::Done | StepState::Skipped)),
            "a completed setup must not leave a step pending or active",
        );
    }

    #[test]
    fn the_log_is_capped_so_a_chatty_stage_cannot_grow_the_snapshot_without_bound() {
        let (tracker, _) = tracker();
        tracker.begin(None);
        for i in 0..(MAX_LOG_LINES + 50) {
            tracker.append_log(None, &format!("line {i}"));
        }
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.log.len(), MAX_LOG_LINES);
        // The cap drops the oldest lines, so the most recent, the ones that explain how a
        // run ended, are the ones kept.
        assert!(snapshot
            .log
            .last()
            .expect("a line")
            .text
            .contains(&format!("line {}", MAX_LOG_LINES + 49)));
    }

    #[test]
    fn a_crashed_run_is_reported_as_interrupted_rather_than_restored_as_running() {
        let dir = temp_dir();
        // A record left by a process that is not us, still claiming to be running.
        let record = PersistedOperation {
            operation_id: "abc".into(),
            phase: SetupPhase::Running,
            stage: Some(SetupStage::InstallingEnvironment),
            started_at: 1,
            updated_at: 2,
            pid: 1, // never this test process
        };
        std::fs::write(
            dir.join(OPERATION_FILE),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();

        let tracker = SetupTracker::new(dir.clone());
        tracker.reconcile(None, true);
        let snapshot = tracker.snapshot();
        assert_eq!(
            snapshot.phase,
            SetupPhase::Interrupted,
            "a dead process's 'running' flag must never be restored as running",
        );
        assert!(snapshot.phase.is_terminal());
    }

    #[test]
    fn a_record_of_a_finished_run_is_not_reported_as_an_interruption() {
        let dir = temp_dir();
        let record = PersistedOperation {
            operation_id: "abc".into(),
            phase: SetupPhase::Completed,
            stage: None,
            started_at: 1,
            updated_at: 2,
            pid: 1,
        };
        std::fs::write(
            dir.join(OPERATION_FILE),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        let tracker = SetupTracker::new(dir.clone());
        tracker.reconcile(None, true);
        assert_eq!(tracker.snapshot().phase, SetupPhase::Idle);
    }

    #[test]
    fn a_missing_or_corrupt_record_leaves_the_tracker_idle_rather_than_failing_startup() {
        let dir = temp_dir();
        std::fs::write(dir.join(OPERATION_FILE), b"{not json").unwrap();
        let tracker = SetupTracker::new(dir.clone());
        tracker.reconcile(None, false);
        assert_eq!(tracker.snapshot().phase, SetupPhase::Idle);
    }

    #[test]
    fn clearing_a_record_that_is_not_there_is_not_an_error() {
        let (tracker, _) = tracker();
        assert!(tracker.clear_record().is_ok());
    }

    #[test]
    fn elapsed_time_stops_being_reported_once_the_run_has_ended() {
        let (tracker, _) = tracker();
        tracker.begin(None);
        let running = tracker.snapshot();
        assert!(running.elapsed_ms(running.started_at + 5_000).is_some());
        tracker.finish(None, SetupPhase::Completed, Some(SetupOutcome::Ready), None);
        let done = tracker.snapshot();
        assert!(
            done.elapsed_ms(done.started_at + 5_000).is_none(),
            "a finished run has a duration, not a still-counting elapsed time",
        );
    }

    #[test]
    fn a_line_naming_a_token_is_dropped_whole_rather_than_partly_masked() {
        assert_eq!(redact("token=abc123def"), "[redacted]");
        assert_eq!(redact("Authorization password: hunter2"), "[redacted]");
        assert_eq!(redact("SECRET_VALUE is set"), "[redacted]");
    }

    #[test]
    fn a_windows_profile_path_keeps_its_shape_but_loses_the_account_name() {
        assert_eq!(
            redact(r"failed to read C:\Users\yassin\AppData\Local\thing.log"),
            r"failed to read C:\Users\<user>\AppData\Local\thing.log",
        );
    }

    #[test]
    fn redaction_leaves_ordinary_lines_untouched() {
        let line = "dism reported exit code 3010";
        assert_eq!(redact(line), line);
    }

    #[test]
    fn every_planned_step_has_an_explanation_written_for_it() {
        for &stage in plan_stages() {
            let text = explain_stage(stage);
            assert!(
                text.len() > 40,
                "{stage:?} needs a real explanation, not a placeholder",
            );
            // The product rule is that infrastructure vocabulary stays out of the UI.
            let lowered = text.to_ascii_lowercase();
            for word in ["wsl", "distro", "tarball", "registry"] {
                assert!(
                    !lowered.contains(word),
                    "{stage:?} explanation leaks the word {word:?} to the user",
                );
            }
        }
    }
}
