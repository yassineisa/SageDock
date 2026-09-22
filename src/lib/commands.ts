/**
 * Single bridge to the Rust backend. Every backend call the UI needs goes through a typed
 * function here — components must never call `invoke` directly, so the set of operations
 * the frontend can trigger stays an explicit, auditable list. Imports use native pickers;
 * notebook paths are workspace-relative and validated by the backend. No call accepts
 * arbitrary shell commands or absolute filesystem paths from the webview.
 */
import { invoke } from "@tauri-apps/api/core";

export type ThemePreference = "system" | "light" | "dark";

export interface AppConfig {
  schema_version: number;
  theme: ThemePreference;
  open_in_browser: boolean;
  /** Whether the first-run introduction has been finished or skipped. */
  onboarding_complete: boolean;
  /** A `Browser.id` from `installedBrowsers`, or null for whatever Windows has set. */
  preferred_browser: string | null;
}

/**
 * One web browser installed on this PC. `id` is a registry client name, not a path — the
 * frontend never learns or sends where a program lives. See `src-tauri/src/browsers.rs`.
 */
export interface Browser {
  id: string;
  name: string;
}

export interface AppInfo {
  name: string;
  version: string;
  developer: string;
  license: string;
}

type ErrorSeverity = "info" | "warning" | "error" | "fatal";

interface RecoveryAction {
  id: string;
  label: string;
}

/** Mirrors the Rust `AppError` shape — see src-tauri/src/error.rs. */
export interface AppError {
  code: string;
  title: string;
  message: string;
  severity: ErrorSeverity;
  user_files_safe: boolean;
  component: string;
  recovery_actions: RecoveryAction[];
  technical_details: string | null;
  timestamp: string;
}

/** Type guard: distinguishes a structured `AppError` from an unexpected throw. */
export function isAppError(value: unknown): value is AppError {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "title" in value &&
    "message" in value &&
    "severity" in value
  );
}

/** Mirrors Rust's `SetupStage` — see src-tauri/src/runtime/provision.rs. */
export type SetupStage =
  | "preflight"
  | "installing_windows_components"
  | "waiting_for_restart"
  | "checking_sage_package"
  | "installing_environment"
  | "creating_workspace"
  | "verifying"
  | "ready";

/** How a setup run ended. Only "ready" means usable; the others are next steps, not errors. */
type SetupOutcome = "ready" | "awaiting_restart" | "needs_sage_package";

/**
 * What setup is doing, at the level the user is asked to act on.
 *
 * Coarser than `SetupStage` on purpose: a stage names the step, a phase says whether
 * anybody needs to do anything. "waiting_for_permission" and "waiting_for_windows" look
 * identical from outside but need completely different words on screen.
 */
export type SetupPhase =
  | "idle"
  | "running"
  | "waiting_for_permission"
  | "waiting_for_windows"
  | "restart_required"
  | "completed"
  | "failed"
  | "interrupted";

export type StepState = "pending" | "active" | "done" | "skipped" | "failed";

export interface SetupStep {
  stage: SetupStage;
  title: string;
  /** Why this step exists, in plain language. */
  explanation: string;
  state: StepState;
}

interface SetupLogLine {
  at: number;
  text: string;
}

/**
 * The authoritative state of setup. Emitted on "setup-progress" and returned by
 * `setupSnapshot()` — deliberately the same shape, so a screen that mounts halfway
 * through recovers exactly what a screen that was listening all along already has.
 */
export interface SetupSnapshot {
  /** Identifies one run. A snapshot from an older run must never replace a newer one. */
  operation_id: string;
  /** Increases on every published change, within an operation. */
  seq: number;
  phase: SetupPhase;
  stage: SetupStage | null;
  title: string;
  detail: string | null;
  /** Only ever measured. `null` means the stage cannot measure itself — show indeterminate
   * progress rather than inventing a number. */
  percent: number | null;
  steps: SetupStep[];
  started_at: number;
  /** When the stage or phase last genuinely changed. Not touched by the heartbeat. */
  updated_at: number;
  /** Proof the backend thread is alive. Explicitly not evidence of progress. */
  heartbeat_at: number;
  outcome: SetupOutcome | null;
  problem: AppError | null;
  log: SetupLogLine[];
}

/** Whether setup is in flight, including while it waits on the user or on Windows. */
export function isActivePhase(phase: SetupPhase): boolean {
  return (
    phase === "running" || phase === "waiting_for_permission" || phase === "waiting_for_windows"
  );
}

/** The SageMath package setup would install from. */
export interface SagePackageInfo {
  file_name: string;
  folder: string;
  sage_version: string | null;
  /** Whether a checksum file was found, so setup can prove the copy is complete. */
  has_checksum: boolean;
}

export interface SetupStatus {
  environment_ready: boolean;
  environment_installed: boolean;
  awaiting_restart: boolean;
  workspace_path: string;
  sage_package: SagePackageInfo | null;
  problem: AppError | null;
  busy: boolean;
}

export type NotebookKind = "sage" | "python";

/** `new_notebook` only creates the file; opening it is what starts the notebook service. */
export interface NotebookLaunch {
  relative_path: string;
}

// --- computing environment ------------------------------------------------------------

/**
 * Observed state of the Linux environment, not a record of what SageDock last did to it.
 * `busy` names the operation in flight, so the UI can disable conflicting actions and say
 * what it's waiting for.
 */
export interface EnvironmentStatus {
  installed: boolean;
  running: boolean | null;
  notebooks_running: boolean;
  open_workspaces: number;
  busy: string | null;
}

// --- workspaces -------------------------------------------------------------------------

/** Why a workspace folder can't be used — each needs different advice, so not a boolean. */
type WorkspaceStatus = "available" | "missing" | "unreadable";

export interface WorkspaceView {
  id: string;
  name: string;
  /** Absolute Windows path, shown so the user knows exactly which folder this is. */
  path: string;
  /** Unix seconds, or null if never launched. */
  last_opened: number | null;
  is_active: boolean;
  status: WorkspaceStatus;
}

// --- backups ------------------------------------------------------------------------------

/** Streamed on the "backup-progress" event, for both backup and restore. */
export interface BackupProgress {
  stage: string;
  detail: string | null;
  percent: number | null;
}

export interface BackupSummary {
  path: string;
  file_count: number;
  total_bytes: number;
  workspaces: string[];
  /** Workspaces whose folder couldn't be read and so aren't in the backup. */
  skipped: string[];
}

interface PreviewWorkspace {
  name: string;
  file_count: number;
  total_bytes: number;
  /** A workspace of this name already exists, so it will be restored alongside it. */
  conflicts: boolean;
  restored_as: string;
}

interface BackupPreview {
  format: number;
  app_version: string;
  created_utc: string;
  file_count: number;
  total_bytes: number;
  workspaces: PreviewWorkspace[];
  compatible: boolean;
  note: string | null;
}

export interface ChosenBackup {
  file_name: string;
  preview: BackupPreview;
}

export interface RestoreSummary {
  restored: string[];
  file_count: number;
}

/** What choosing a workspace for a downloaded notebook would do. See `checkDownloadTarget`. */
export interface DownloadTarget {
  workspace_name: string;
  /** The file name the notebook would land as, before any dedup numbering. */
  target_name: string;
  /** Whether that name is already taken in the chosen workspace. */
  exists: boolean;
}

/** What to do about a name already in use. See `home::DownloadConflict` in Rust. */
export type DownloadConflict = "open_existing" | "replace" | "copy";

/**
 * Result of adding a native drop to a selected card. Source paths stay in Rust.
 */
export interface FilesDropped {
  workspace: string;
  added: number;
  failed: number;
  error: string | null;
}

export const commands = {
  addDroppedFiles: (dropId: string, id: string | null) =>
    invoke<FilesDropped | null>("add_dropped_files", { dropId, id }),
  getAppInfo: () => invoke<AppInfo>("get_app_info"),
  /** Opens the project page in the default browser. The address is fixed in Rust, not here. */
  openProjectPage: () => invoke<void>("open_project_page"),
  getConfig: () => invoke<AppConfig>("get_config"),
  setTheme: (theme: ThemePreference) => invoke<AppConfig>("set_theme", { theme }),
  /** Browsers Windows reports as installed, for the first-run choice and Settings. */
  installedBrowsers: () => invoke<Browser[]>("installed_browsers"),
  /** `null` restores "whatever Windows has set as the default". */
  setPreferredBrowser: (value: string | null) =>
    invoke<AppConfig>("set_preferred_browser", { value }),
  /** `false` makes the introduction appear again on the next launch. */
  setOnboardingComplete: (value: boolean) =>
    invoke<AppConfig>("set_onboarding_complete", { value }),
  getSetupStatus: () => invoke<SetupStatus>("get_setup_status"),
  /** Opens a native file picker in the backend. Resolves to null if the user closes it. */
  chooseSagePackage: () => invoke<SagePackageInfo | null>("choose_sage_package"),
  /**
   * Starts setup and returns immediately with the opening snapshot.
   *
   * It deliberately does not resolve when setup *finishes*: setup belongs to the backend
   * and outlives whichever screen started it. Watch `setup-progress` or poll
   * `setupSnapshot()` for the rest.
   */
  runSetup: () => invoke<SetupSnapshot>("run_setup"),
  /** The authoritative state of setup, answerable at any moment. */
  setupSnapshot: () => invoke<SetupSnapshot>("setup_snapshot"),
  /** Marks an interrupted run as seen, so it is reported once rather than every launch. */
  acknowledgeSetupInterruption: () => invoke<void>("acknowledge_setup_interruption"),
  /** A shareable report of the setup run, already redacted. */
  setupDiagnostics: () => invoke<string>("setup_diagnostics"),
  newNotebook: (kind: NotebookKind) => invoke<NotebookLaunch>("new_notebook", { kind }),
  /** Stops SageMath and closes the app. The promise may never settle — the process exits. */
  shutdownAndQuit: () => invoke<void>("shutdown_and_quit"),
  /** Opens the folder holding every workspace, not whichever one is currently active. */
  openWorkspacesFolder: () => invoke<void>("open_workspaces_folder"),
  recentNotebooks: () => invoke<NotebookEntry[]>("recent_notebooks"),
  /** Notebooks found in the Windows Downloads folder. `path` is a bare file name. */
  downloadedNotebooks: () => invoke<NotebookEntry[]>("downloaded_notebooks"),
  /**
   * What choosing this workspace for a downloaded notebook would do, without doing it.
   * Called right after the student picks a workspace, purely to decide whether to ask
   * about a name collision before anything is copied.
   */
  checkDownloadTarget: (name: string, workspaceId: string) =>
    invoke<DownloadTarget>("check_download_target", { name, workspaceId }),
  /**
   * Copies a downloaded notebook into the chosen workspace and opens it there, making that
   * workspace active — the same as Launch workspace on its card. Opening one in place is
   * impossible: the notebook service is rooted at a workspace, not at Downloads.
   */
  openDownloadedNotebook: (name: string, workspaceId: string, onConflict: DownloadConflict) =>
    invoke<string>("open_downloaded_notebook", { name, workspaceId, onConflict }),
  /**
   * Starts a native drag so a notebook can be dropped on the desktop or a folder.
   *
   * Only the same relative name the backend handed out is sent back; the absolute path is
   * resolved in Rust, exactly like every other notebook operation. The drag copies.
   */
  startNotebookDrag: (path: string, source: "workspace" | "downloads") =>
    invoke<void>("start_notebook_drag", { path, source }),
  /**
   * Opens a native picker for a notebook to add to a workspace. Nothing is copied yet — the
   * backend only remembers the choice — so it can be run through the same workspace-and-
   * conflict flow as a notebook found in Downloads. Resolves to null if the picker is closed.
   */
  chooseNotebookFile: () => invoke<string | null>("choose_notebook_file"),
  /** What choosing this workspace for the notebook picked via `chooseNotebookFile` would do. */
  checkPickedTarget: (workspaceId: string) =>
    invoke<DownloadTarget>("check_picked_target", { workspaceId }),
  /** Copies the notebook picked via `chooseNotebookFile` into the chosen workspace and opens it. */
  openPickedNotebook: (workspaceId: string, onConflict: DownloadConflict) =>
    invoke<string>("open_picked_notebook", { workspaceId, onConflict }),
  /** Moves a notebook already in the active workspace to the Recycle Bin. */
  deleteRecentNotebook: (path: string) => invoke<void>("delete_recent_notebook", { path }),
  /** Reveals a notebook already in the active workspace in File Explorer, selected. */
  revealRecentNotebook: (path: string) => invoke<void>("reveal_recent_notebook", { path }),
  /** Moves a file in Downloads to the Recycle Bin. */
  deleteDownloadedFile: (name: string) => invoke<void>("delete_downloaded_file", { name }),
  /** Opens a downloaded file with its default Windows program, without adding it to SageDock. */
  openDownloadedFileExternally: (name: string) =>
    invoke<void>("open_downloaded_file_externally", { name }),
  /** Reveals a downloaded file in File Explorer, selected. */
  revealDownloadedFile: (name: string) => invoke<void>("reveal_downloaded_file", { name }),
  openNotebook: (path: string) => invoke<void>("open_notebook", { path }),
  /**
   * Opens JupyterLab's landing page, rooted at the default SageDock folder. Deliberately
   * separate from `openNotebook("")`: that addresses the file browser at a session root and
   * follows whichever course was launched last.
   */
  openJupyterHome: () => invoke<void>("open_jupyter_home"),
  setBrowserPreference: (value: boolean) => invoke<AppConfig>("set_browser_preference", { value }),
  /** `force` is the user pressing "Check now"; without it a stopped environment stays stopped. */
  scientificTools: (force: boolean) => invoke<ToolReport>("scientific_tools", { force }),
  /** Installs or repairs. Streams `tool-progress` and returns the freshly verified report. */
  installScientificTool: (tool: ToolId) => invoke<ToolReport>("install_scientific_tool", { tool }),
  recover: (action: RecoveryId) => invoke<string>("recover", { action }),
  diagnosticReport: () => invoke<string>("diagnostic_report"),
  saveDiagnosticReport: () => invoke<boolean>("save_diagnostic_report"),
  restartWindows: () => invoke<void>("restart_windows"),

  // Computing environment
  environmentStatus: () => invoke<EnvironmentStatus>("environment_status"),
  /** Stops notebooks then SageDock's own distribution. Returns the state actually observed. */
  stopEnvironment: () => invoke<EnvironmentStatus>("stop_environment"),

  // Workspaces
  listWorkspaces: () => invoke<WorkspaceView[]>("list_workspaces"),
  createWorkspace: (name: string) => invoke<WorkspaceView>("create_workspace", { name }),
  /** Native folder picker. Resolves to null if the user closes it. */
  addWorkspaceFolder: () => invoke<WorkspaceView | null>("add_workspace_folder"),
  renameWorkspace: (id: string, name: string) =>
    invoke<WorkspaceView>("rename_workspace", { id, name }),
  /** Native multi-file picker. Copies the chosen files in; never overwrites. */
  addFilesToWorkspace: (id: string) => invoke<number>("add_files_to_workspace", { id }),
  /** Removes it from the list only. The folder and its files are never deleted. */
  forgetWorkspace: (id: string) => invoke<WorkspaceView[]>("forget_workspace", { id }),
  launchWorkspace: (id: string) => invoke<void>("launch_workspace", { id }),
  revealWorkspace: (id: string) => invoke<void>("reveal_workspace", { id }),

  // Backups
  createBackup: () => invoke<BackupSummary | null>("create_backup"),
  previewBackup: () => invoke<ChosenBackup | null>("preview_backup"),
  restoreBackup: () => invoke<RestoreSummary>("restore_backup"),
};

export interface NotebookEntry {
  path: string;
  name: string;
  modified: number;
  /**
   * Where the file is, for a download saved outside the Windows Downloads folder — the
   * containing folder's **name** only, never a path. `null` means the Downloads folder
   * itself. Recently-opened notebooks never set it.
   */
  folder?: string | null;
  /**
   * Whether this is a Jupyter notebook, and so can be added to a workspace. The built-in
   * browser downloads any file type, and a downloaded PDF has nothing to add a workspace
   * to. Absent means notebook, which is what every other list returns.
   */
  notebook?: boolean;
}
export type ToolId = "cpp" | "fortran" | "build" | "toolkit" | "seaborn" | "statsmodels" | "polars";

/**
 * Mirrors Rust's `scientific::ToolState`.
 *
 * `needs_repair` is the important one: a group that is only partly installed, or a compiler
 * that exists but cannot actually build anything. It is neither "installed" nor "not
 * installed", and collapsing it into either misleads the student.
 *
 * Not exported, like `SetupStage` and `WorkspaceStatus` above: it is reached through
 * `ToolStatus`, and exporting a type nothing imports is what the unused-export scan flags.
 */
type ToolState = "installed" | "needs_repair" | "not_installed";

/**
 * Where a report came from. `cached` and `unavailable` must never be rendered as a negative
 * answer: not knowing whether a compiler is installed is not the same as it being missing.
 */
type ReportSource = "verified" | "cached" | "unavailable";

/** One executable that has to exist *and* work. See `scientific::ComponentStatus`. */
interface ComponentStatus {
  id: string;
  label: string;
  /** Found on PATH. Necessary, never sufficient. */
  present: boolean;
  /** Compiled and ran a test program, or did its real job. */
  works: boolean;
  version: string | null;
  detail: string | null;
}

export interface ToolStatus {
  id: ToolId;
  state: ToolState;
  version: string | null;
  components: ComponentStatus[];
}

export interface ToolReport {
  source: ReportSource;
  checked_at: string | null;
  reason: string | null;
  tools: ToolStatus[];
}

/** Streamed on the "tool-progress" event during an install or repair. */
export interface ToolProgress {
  stage: "checking" | "installing" | "verifying" | "done";
  title: string;
  detail: string | null;
  percent: number | null;
}
export type RecoveryId = "service" | "environment" | "verify" | "rebuild" | "restore";

export function friendlyError(error: unknown): {
  title: string;
  message: string;
  details?: string | null;
} {
  return isAppError(error)
    ? { title: error.title, message: error.message, details: error.technical_details }
    : {
        title: "SageDock couldn't finish this action",
        message:
          "Your saved notebooks are safe. Try again, or open Recovery to check your environment.",
      };
}

/** Human-readable byte size for backup previews. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} bytes`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

/** "Last opened" wording that stays plain: today, yesterday, then a date. */
export function formatLastOpened(seconds: number | null): string {
  if (!seconds) return "Not opened yet";
  const then = new Date(seconds * 1000);
  const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((startOfDay(new Date()) - startOfDay(then)) / 86_400_000);
  if (days <= 0) return "Opened today";
  if (days === 1) return "Opened yesterday";
  if (days < 7) return `Opened ${days} days ago`;
  return `Opened ${then.toLocaleDateString(undefined, { month: "short", day: "numeric" })}`;
}
