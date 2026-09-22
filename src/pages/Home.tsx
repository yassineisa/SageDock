import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Link } from "react-router-dom";
import {
  commands,
  formatBytes,
  friendlyError,
  isActivePhase,
  isAppError,
  type BackupProgress,
  type ChosenBackup,
  type DownloadConflict,
  type EnvironmentStatus,
  type FilesDropped,
  type NotebookEntry,
  type NotebookKind,
  type SetupStatus,
  type ToolReport,
  type WorkspaceView,
} from "../lib/commands";
import { useTask } from "../state/TaskContext";
import { useSetup } from "../state/SetupContext";
import { SetupProgress } from "../components/SetupProgress";
import { SetupExplainer } from "../components/SetupExplainer";
import { ErrorBanner } from "../components/ErrorBanner";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { ChoiceDialog, type ChoiceOption } from "../components/ChoiceDialog";
import { WorkspaceCard } from "../components/WorkspaceCard";
import { NotebookRow } from "../components/NotebookRow";
import { Icon, type IconName } from "../components/Icon";

/**
 * What Home summarises. Deliberately the three real groups and not the derived "full
 * toolkit", so the strip stays compact and every chip maps to something the Tools page can
 * act on.
 */
const CAPABILITIES: { id: "cpp" | "fortran" | "build"; name: string; icon: IconName }[] = [
  { id: "cpp", name: "C / C++", icon: "code" },
  { id: "fortran", name: "Fortran", icon: "numeric" },
  { id: "build", name: "Build tools", icon: "buildTools" },
];

export function Home() {
  const task = useTask();
  const setup = useSetup();
  const [status, setStatus] = useState<SetupStatus | null>(null);
  const [environment, setEnvironment] = useState<EnvironmentStatus | null>(null);
  const [workspaces, setWorkspaces] = useState<WorkspaceView[]>([]);
  const [loadError, setLoadError] = useState<ReturnType<typeof friendlyError> | null>(null);
  const [transfer, setTransfer] = useState<BackupProgress | null>(null);
  // Opens the dedicated explanation. A modal rather than a route: navigating to /help
  // mid-setup was both the wrong answer to the question and the surest way to lose the
  // screen that was showing progress.
  const [explaining, setExplaining] = useState(false);
  const [recent, setRecent] = useState<NotebookEntry[]>([]);
  // Notebooks sitting in the Windows Downloads folder. Kept separate from `recent` because
  // they are not in a workspace yet: opening one copies it in first.
  const [downloads, setDownloads] = useState<NotebookEntry[]>([]);
  const [search, setSearch] = useState("");
  const [restart, setRestart] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [forgetting, setForgetting] = useState<WorkspaceView | null>(null);
  const [pendingRestore, setPendingRestore] = useState<ChosenBackup | null>(null);
  // Null means SageDock has not been able to look, which is deliberately not the same fact
  // as a report saying nothing is installed.
  const [tools, setTools] = useState<ToolReport | null>(null);
  const [dropping, setDropping] = useState<string | null>(null);
  // Renaming was refused because a notebook is open in that folder. Shown as a pop-up
  // rather than a banner: it interrupts something the user just asked for, and the reason
  // is actionable.
  const [renameBlocked, setRenameBlocked] = useState<{ name: string; message: string } | null>(
    null,
  );

  // Each read is settled independently: one failing call must not blank the whole screen
  // or hide backup and folder management, which stay usable even when the runtime doesn't.
  async function refresh() {
    setLoadError(null);
    try {
      const [next, files, spaces, env, kit, inbox] = await Promise.allSettled([
        commands.getSetupStatus(),
        commands.recentNotebooks(),
        commands.listWorkspaces(),
        commands.environmentStatus(),
        // `false` is the whole point: reading Home must never start an environment the user
        // explicitly stopped. The backend returns the last known result instead, labelled.
        commands.scientificTools(false),
        commands.downloadedNotebooks(),
      ]);
      if (next.status === "fulfilled") setStatus(next.value);
      if (files.status === "fulfilled") setRecent(files.value);
      else setRecent([]);
      if (spaces.status === "fulfilled") setWorkspaces(spaces.value);
      if (env.status === "fulfilled") setEnvironment(env.value);
      // Left out of the error banner below on purpose: not knowing which compilers are
      // present is worth showing in place, but it is not a reason to put an error across
      // the top of Home and it must never read as "not installed".
      setTools(kit.status === "fulfilled" ? kit.value : null);
      // Downloads are a convenience, not part of anyone's workspace: being unable to read
      // that folder is not a reason to put an error across Home either.
      setDownloads(inbox.status === "fulfilled" ? inbox.value : []);
      const failed = [next, files, spaces, env].find((result) => result.status === "rejected");
      if (failed?.status === "rejected") setLoadError(friendlyError(failed.reason));
    } catch (e) {
      setLoadError(friendlyError(e));
    }
  }

  useEffect(() => {
    if (!task.busy) void refresh();
  }, [task.busy]);

  // Setup itself is no longer subscribed to here. It belongs to `SetupProvider`, which
  // outlives this screen, Home is one of several observers and holds none of the state.
  // What Home does still need is to re-read *its own* data when setup reaches an end, so
  // the workspace list and environment status reflect the newly installed runtime.
  const setupPhase = setup.snapshot?.phase;
  useEffect(() => {
    if (setupPhase === "completed" || setupPhase === "failed") void refresh();
  }, [setupPhase]);

  useEffect(() => {
    const sub = listen<BackupProgress>("backup-progress", (e) => setTransfer(e.payload));
    return () => {
      sub.then((f) => f()).catch(() => {});
    };
  }, []);

  // What the last drop from Windows did. Declared beside the effect that sets it.
  const [dropped, setDropped] = useState<FilesDropped | null>(null);

  // Lets a student know when something new lands in Downloads, the built-in Sage browser
  // saves there like any other browser would. There is no stable hook into WebView2's own
  // download plumbing, so this polls the same folder the Downloads section already reads
  // and reports the difference. A short interval rather than an instant push is a fair
  // trade for not depending on undocumented webview internals.
  useEffect(() => {
    let seen: Set<string> | null = null;
    const poll = async () => {
      let found: NotebookEntry[];
      try {
        found = await commands.downloadedNotebooks();
      } catch {
        return; // Downloads being briefly unreadable isn't worth interrupting anything over.
      }
      if (seen) {
        const fresh = found.filter((f) => !seen!.has(f.path));
        if (fresh.length === 1) {
          task.notify(`${fresh[0].name} was downloaded. Find it in Downloads.`);
        } else if (fresh.length > 1) {
          task.notify(`${fresh.length} new notebooks were downloaded. Find them in Downloads.`);
        }
      }
      seen = new Set(found.map((f) => f.path));
    };
    const interval = setInterval(() => void poll(), 8000);
    return () => clearInterval(interval);
  }, []);

  // Resolve both hover and final drop against the actual card under the pointer. Native
  // coordinates are physical pixels; elementFromPoint uses CSS pixels (including zoom).
  useEffect(() => {
    let disposed = false;
    const subscription = listen<{
      type: "over" | "drop" | "leave";
      position?: { x: number; y: number };
      drop_id?: string;
    }>("workspace-drag", (e) => {
      if (disposed) return;
      const { type, position, drop_id } = e.payload;
      const card = position
        ? document
            .elementFromPoint(
              position.x / window.devicePixelRatio,
              position.y / window.devicePixelRatio,
            )
            ?.closest<HTMLElement>("[data-workspace-drop]")
        : null;
      const id = card?.dataset.workspaceDrop ?? null;
      setDropping(type === "over" ? id : null);
      if (type === "drop" && drop_id) {
        void task.run("Adding dropped files", async () => {
          const outcome = await commands.addDroppedFiles(drop_id, id);
          if (!disposed) {
            setDropped(outcome);
            if (outcome?.added) await refresh();
          }
        });
      }
    });
    return () => {
      disposed = true;
      subscription.then((off) => off()).catch(() => {});
    };
  }, []);

  // Setup is no longer part of `task.busy`: it is owned by the backend and reported by
  // its own snapshot, so a running setup disables the rest of Home without the frontend
  // having to hold a promise open to remember it.
  const setupActive = !!setup.snapshot && isActivePhase(setup.snapshot.phase);
  const busy = !!task.busy || !!status?.busy || setupActive;
  const ready = status?.environment_ready && !status.problem;
  const running = !!environment?.running;
  // Three states, not two: a failed status query must not be reported as "stopped".
  const runningUnknown = environment?.running == null;

  const start = () => void setup.start();

  const create = (kind: NotebookKind) =>
    void task.run("Opening notebook", async () => {
      const result = await commands.newNotebook(kind);
      await commands.openNotebook(result.relative_path);
      await refresh();
    });

  const open = (path: string) =>
    void task.run("Opening notebook", () => commands.openNotebook(path));

  // JupyterLab's own landing page. Deliberately not `openNotebook("")`, which addresses the
  // file browser at whichever course folder was launched last: this button is the general
  // "start working" action and must not quietly become course-specific.
  const openJupyter = () =>
    void task.run("Opening JupyterLab", async () => {
      await commands.openJupyterHome();
      await refresh();
    });

  // The one path that may start a stopped environment, because the user asked for it.
  const checkTools = () =>
    void task.run("Checking installed tools", async () => {
      setTools(await commands.scientificTools(true));
    });

  const importFile = () =>
    void task.run("Choosing a notebook", async () => {
      const name = await commands.chooseNotebookFile();
      // The workspace-and-conflict dialogs decide what happens next; nothing is copied yet.
      if (name) setPendingImport({ kind: "picked", displayName: name });
    });

  const stopEnvironment = () => {
    setStopping(false);
    void task.run("Stopping SageMath", async () => {
      const next = await commands.stopEnvironment();
      setEnvironment(next);
      await refresh();
      return "SageMath has stopped. Opening a notebook or workspace will start it again.";
    });
  };

  const createWorkspace = () => {
    const name = newName.trim();
    if (!name) return;
    void task.run("Creating workspace", async () => {
      await commands.createWorkspace(name);
      setNewName("");
      setCreating(false);
      await refresh();
      return `${name} is ready. Choose Launch workspace to start working in it.`;
    });
  };

  const addFolder = () =>
    void task.run("Adding folder", async () => {
      const added = await commands.addWorkspaceFolder();
      if (!added) return;
      await refresh();
      return `${added.name} was added to your workspaces.`;
    });

  const launch = (workspace: WorkspaceView) =>
    void task.run(`Opening ${workspace.name}`, async () => {
      await commands.launchWorkspace(workspace.id);
      await refresh();
    });

  const rename = (workspace: WorkspaceView, name: string) =>
    void task.run("Renaming workspace", async () => {
      try {
        await commands.renameWorkspace(workspace.id, name);
      } catch (err) {
        // Renaming moves the folder on disk, so a notebook open inside that folder blocks
        // it. That deserves a pop-up rather than a banner: the user has just typed a new
        // name and pressed Save, and needs to know why it didn't take and what to do.
        if (isAppError(err) && err.code === "WORKSPACE_IN_USE") {
          setRenameBlocked({ name: workspace.name, message: err.message });
          return;
        }
        throw err;
      }
      await refresh();
    });

  const addFiles = (workspace: WorkspaceView) =>
    void task.run(`Adding files to ${workspace.name}`, async () => {
      const added = await commands.addFilesToWorkspace(workspace.id);
      if (!added) return;
      await refresh();
      return `${added} file(s) copied into ${workspace.name}. Nothing already there was replaced.`;
    });

  const reveal = (workspace: WorkspaceView) =>
    void task.run("Opening folder", () => commands.revealWorkspace(workspace.id));

  // A notebook from Downloads or the "Open notebook" picker could belong to any workspace,
  // so opening either is a two-step choice rather than a single click: which workspace,
  // then, only if that workspace already has a notebook by this name, what to do about
  // it. Both sources share this flow, so a notebook behaves the same regardless of where it
  // came from.
  const [pendingImport, setPendingImport] = useState<
    | { kind: "download"; displayName: string; path: string }
    | { kind: "picked"; displayName: string }
    | null
  >(null);
  const [importConflict, setImportConflict] = useState<
    | {
        kind: "download";
        path: string;
        workspaceId: string;
        workspaceName: string;
        targetName: string;
      }
    | { kind: "picked"; workspaceId: string; workspaceName: string; targetName: string }
    | null
  >(null);

  const chooseDownloadWorkspace = (entry: NotebookEntry) =>
    setPendingImport({ kind: "download", displayName: entry.name, path: entry.path });

  const pickWorkspaceForImport = (workspaceId: string) => {
    const pending = pendingImport;
    setPendingImport(null);
    if (!pending) return;
    void task.run("Checking your workspace", async () => {
      const check =
        pending.kind === "download"
          ? await commands.checkDownloadTarget(pending.path, workspaceId)
          : await commands.checkPickedTarget(workspaceId);
      if (check.exists) {
        // Nothing is copied yet: the conflict dialog decides what happens next.
        setImportConflict(
          pending.kind === "download"
            ? {
                kind: "download",
                path: pending.path,
                workspaceId,
                workspaceName: check.workspace_name,
                targetName: check.target_name,
              }
            : {
                kind: "picked",
                workspaceId,
                workspaceName: check.workspace_name,
                targetName: check.target_name,
              },
        );
        return;
      }
      const landed =
        pending.kind === "download"
          ? await commands.openDownloadedNotebook(pending.path, workspaceId, "copy")
          : await commands.openPickedNotebook(workspaceId, "copy");
      await refresh();
      return `${landed} is open in ${check.workspace_name}.`;
    });
  };

  const resolveImportConflict = (choice: string) => {
    const conflict = importConflict;
    setImportConflict(null);
    if (!conflict) return;
    void task.run("Opening notebook", async () => {
      const landed =
        conflict.kind === "download"
          ? await commands.openDownloadedNotebook(
              conflict.path,
              conflict.workspaceId,
              choice as DownloadConflict,
            )
          : await commands.openPickedNotebook(conflict.workspaceId, choice as DownloadConflict);
      await refresh();
      return `${landed} is open in ${conflict.workspaceName}.`;
    });
  };

  // Deleting sends a notebook to the Recycle Bin rather than unlinking it, so a mistaken
  // click doesn't cost real work, the same margin Explorer itself gives.
  const [deletingRecent, setDeletingRecent] = useState<NotebookEntry | null>(null);
  const confirmDeleteRecent = () => {
    const entry = deletingRecent;
    setDeletingRecent(null);
    if (!entry) return;
    void task.run("Deleting notebook", async () => {
      await commands.deleteRecentNotebook(entry.path);
      await refresh();
      return `${entry.name} was moved to the Recycle Bin.`;
    });
  };
  const revealRecent = (entry: NotebookEntry) =>
    void task.run("Opening folder", () => commands.revealRecentNotebook(entry.path));

  const [deletingDownload, setDeletingDownload] = useState<NotebookEntry | null>(null);
  const confirmDeleteDownload = () => {
    const entry = deletingDownload;
    setDeletingDownload(null);
    if (!entry) return;
    void task.run("Deleting download", async () => {
      await commands.deleteDownloadedFile(entry.path);
      setDownloads((prev) => prev.filter((d) => d.path !== entry.path));
      return `${entry.name} was moved to the Recycle Bin.`;
    });
  };
  // Opens with whatever Windows already associates with the file, a quick look, not an
  // import, so a student can check a download without deciding where it belongs yet.
  const openDownloadExternally = (entry: NotebookEntry) =>
    void task.run("Opening file", () => commands.openDownloadedFileExternally(entry.path));
  const revealDownload = (entry: NotebookEntry) =>
    void task.run("Opening folder", () => commands.revealDownloadedFile(entry.path));

  // Downloads change outside SageDock, a browser writes a file while Home is already on
  // screen, so this re-reads just that folder rather than reloading the whole page.
  const refreshDownloads = () =>
    void task.run("Checking your Downloads folder", async () => {
      const found = await commands.downloadedNotebooks();
      setDownloads(found);
      return found.length
        ? `Found ${found.length} notebook(s) in your Downloads folder.`
        : "No notebooks found in your Downloads folder.";
    });

  // Hands the drag to Windows so the notebook can be dropped on the desktop or a folder.
  // `preventDefault` stops the webview also starting its own drag of the row's text.
  const dragOut =
    (path: string, source: "workspace" | "downloads") =>
    (event: { preventDefault: () => void }) => {
      event.preventDefault();
      void commands.startNotebookDrag(path, source).catch(() => {});
    };

  const confirmForget = () => {
    const workspace = forgetting;
    setForgetting(null);
    if (!workspace) return;
    void task.run("Updating workspaces", async () => {
      await commands.forgetWorkspace(workspace.id);
      await refresh();
      return `${workspace.name} was removed from the list. Its folder and files are still on your PC.`;
    });
  };

  const backUp = () =>
    void task.run("Creating backup", async () => {
      const summary = await commands.createBackup();
      setTransfer(null);
      if (!summary) return;
      const skipped = summary.skipped.length
        ? ` ${summary.skipped.length} workspace(s) could not be read and were left out: ${summary.skipped.join(", ")}.`
        : "";
      return `Backup saved and checked. ${summary.file_count} file(s), ${formatBytes(summary.total_bytes)}.${skipped}`;
    });

  const chooseRestore = () =>
    void task.run("Opening backup", async () => {
      const chosen = await commands.previewBackup();
      if (chosen) setPendingRestore(chosen);
    });

  const confirmRestore = () => {
    setPendingRestore(null);
    void task.run("Restoring backup", async () => {
      const summary = await commands.restoreBackup();
      setTransfer(null);
      await refresh();
      return `Restored ${summary.restored.length} workspace(s): ${summary.restored.join(", ")}.`;
    });
  };

  const found = recent.filter((n) =>
    n.name.toLocaleLowerCase().includes(search.toLocaleLowerCase()),
  );
  // Capped so the list stays a glance rather than a scroll. Search still reaches everything
  // SageDock fetched, not just what's shown by default.
  const visibleRecent = search ? found : found.slice(0, 6);

  // Options for the workspace-picker dialog. Unavailable folders stay listed, disabled and
  // explained, the same way WorkspaceCard disables Launch workspace for them, a student
  // should see a workspace exists even on the one visit it can't be used.
  const workspaceOptions: ChoiceOption[] = workspaces.map((w) => ({
    key: w.id,
    label: w.name,
    detail: w.status === "available" ? w.path : "This folder isn't available right now",
    disabled: w.status !== "available",
    icon: "folder",
  }));

  const envLabel = runningUnknown
    ? "Computing environment status unavailable"
    : running
      ? "Computing environment running"
      : "Computing environment stopped";

  return (
    <>
      <header className="page-head">
        <div>
          <h1>Home</h1>
          <p>Your courses, notebooks, and backups.</p>
        </div>
      </header>

      {loadError && (
        <div>
          <ErrorBanner
            title={loadError.title}
            message={loadError.message}
            technicalDetails={loadError.details}
          />
          <div className="btn-row" style={{ marginTop: "var(--sp-3)" }}>
            <button className="btn" onClick={() => void refresh()}>
              <Icon name="refresh" size={16} />
              Check again
            </button>
          </div>
        </div>
      )}

      {!status && !loadError && (
        <div className="task-bar" role="status">
          <span className="spinner" />
          Checking SageMath…
        </div>
      )}

      {status?.problem && (
        <div>
          <ErrorBanner
            title={status.problem.title}
            message={status.problem.message}
            technicalDetails={status.problem.technical_details}
          />
          <div className="btn-row" style={{ marginTop: "var(--sp-3)" }}>
            <Link className="btn btn-accent" to="/recovery">
              <Icon name="repair" size={16} />
              Open Recovery
            </Link>
          </div>
        </div>
      )}

      {/* Deliberately outside the "needs setup" card below. That card is hidden once the
          environment is ready or a health problem is showing, and a setup run has to stay
          visible through both, including the moment it succeeds, when `ready` flips and
          the card disappears out from under it. */}
      {setup.snapshot && setup.snapshot.phase !== "idle" && (
        <SetupProgress snapshot={setup.snapshot} onExplain={() => setExplaining(true)} />
      )}

      {/* A run that never recorded an ending. Reported once, with the honest reason and a
          next step, rather than silently resumed or silently forgotten. */}
      {setup.snapshot?.phase === "interrupted" && (
        <div className="btn-row" style={{ marginTop: "var(--sp-3)" }}>
          <button className="btn btn-accent" disabled={busy || setup.starting} onClick={start}>
            <Icon name="refresh" size={16} />
            Continue setup
          </button>
          <button className="btn btn-subtle" onClick={() => void setup.acknowledgeInterruption()}>
            Dismiss
          </button>
        </div>
      )}

      {/* One card for the environment's status and the two actions that act on it, rather
          than a separate "Start working" card repeating the same context above a second,
          plainer one. Tied to the environment being installed, not to setup being complete,
          so Stop stays reachable even when a health check fails; Open JupyterLab only
          appears once SageDock has actually verified it's ready. */}
      {status?.environment_installed && (
        <section className="env-strip" aria-label="Computing environment">
          <span
            className={`env-dot${running ? " is-running" : ""}${runningUnknown ? " is-unknown" : ""}`}
          />
          <div className="env-text">
            <span className="env-state">
              <span>{envLabel}</span>
              <button
                type="button"
                className="info-tip"
                aria-label="What this means"
                title={
                  runningUnknown
                    ? "SageDock couldn't check whether SageMath is running. You can retry the check or use Stop SageMath. Your saved files are safe."
                    : running
                      ? "SageMath is using memory. Stopping it frees that memory and ends running calculations. SageDock stays open and starts it again when you open a notebook."
                      : "SageMath isn't using memory. It starts again when you open a notebook or launch a workspace."
                }
              >
                <Icon name="info" size={14} />
              </button>
            </span>
          </div>
          <div className="env-actions">
            {ready && (
              <button className="btn btn-accent" disabled={busy} onClick={openJupyter}>
                <Icon name="openFile" size={16} />
                Open JupyterLab
              </button>
            )}
            <button
              className="btn"
              disabled={busy || (!running && !runningUnknown)}
              onClick={() => setStopping(true)}
            >
              <Icon name="stop" size={16} />
              Stop SageMath
            </button>
          </div>
        </section>
      )}

      {/* Hidden while a run is in flight. The progress panel above already says what is
          happening and offers the explanation, so leaving this up put a disabled "Set up
          SageDock" and a second identical "What happens during setup?" on the same screen,
          two controls for one thing, one of them dead. It returns for a failed or
          interrupted run, which is exactly when the retry button is wanted again. */}
      {status && !ready && !status.problem && !setupActive && (
        <section className="card">
          <div className="setup-head">
            <Icon name="info" size={20} />
            <div>
              <h2>
                {status.awaiting_restart
                  ? "Windows needs a restart"
                  : status.environment_installed
                    ? "Finish setting up SageMath"
                    : "Set up SageMath"}
              </h2>
              <p style={{ marginTop: "var(--sp-1)", maxWidth: "70ch" }}>
                {status.awaiting_restart
                  ? "Save your work in other apps, restart Windows, then reopen SageDock and choose Continue setup. Your progress is saved."
                  : "SageDock installs SageMath and Python, then checks that they can run notebooks. Your files stay in Windows."}
              </p>
            </div>
          </div>

          <div className="setup-facts">
            <span className="fact">
              <Icon name="accept" size={14} />
              SageMath and scientific Python
            </span>
            <span className="fact">
              <Icon name="accept" size={14} />
              No terminal setup
            </span>
            <span className="fact">
              <Icon name="accept" size={14} />
              Files stay on your computer
            </span>
          </div>

          <p className="small">
            Needs about 15 GB free. Windows may ask for permission, an internet connection, and a
            restart.
          </p>

          {setup.startError && (
            <p className="small" role="status" style={{ marginTop: "var(--sp-3)" }}>
              {setup.startError}
            </p>
          )}

          <div className="btn-row" style={{ marginTop: "var(--sp-4)" }}>
            {status.awaiting_restart ? (
              <>
                <button className="btn btn-accent" disabled={busy} onClick={() => setRestart(true)}>
                  Restart Windows
                </button>
                <button className="btn" disabled={busy || setup.starting} onClick={start}>
                  Continue setup
                </button>
              </>
            ) : (
              <button className="btn btn-accent" disabled={busy || setup.starting} onClick={start}>
                {setup.snapshot?.phase === "interrupted"
                  ? "Continue setup"
                  : status.environment_installed
                    ? "Finish setup"
                    : "Set up SageDock"}
              </button>
            )}
            {/* Opens the explanation in place. It used to link to /help, which answered a
                different question and navigated away from the progress it was beside. */}
            <button type="button" className="btn btn-subtle" onClick={() => setExplaining(true)}>
              What happens during setup?
            </button>
          </div>

          {!status.sage_package && !status.environment_installed && (
            <div
              style={{
                marginTop: "var(--sp-4)",
                paddingTop: "var(--sp-4)",
                borderTop: "1px solid var(--stroke)",
              }}
            >
              <p className="small">
                The SageMath package wasn't found. If you received it separately, choose it here. A
                complete SageDock installer includes this file.
              </p>
              <div className="btn-row" style={{ marginTop: "var(--sp-3)" }}>
                <button
                  className="btn"
                  disabled={busy}
                  onClick={() =>
                    void task.run("Choosing SageMath package", async () => {
                      await commands.chooseSagePackage();
                      await refresh();
                    })
                  }
                >
                  Choose SageMath package
                </button>
              </div>
            </div>
          )}
        </section>
      )}

      {ready && (
        <section aria-label="Create a notebook">
          <div className="command-bar">
            <button className="btn" disabled={busy} onClick={() => create("sage")}>
              <Icon name="calculator" size={16} />
              New Sage Notebook
            </button>
            <button className="btn" disabled={busy} onClick={() => create("python")}>
              <Icon name="document" size={16} />
              New Python Notebook
            </button>
            <span className="spacer" />
            <button className="btn btn-subtle" disabled={busy} onClick={importFile}>
              <Icon name="openFile" size={16} />
              Open notebook
            </button>
          </div>
        </section>
      )}

      {/* Workspaces and backups are always available, including before setup: a student
          restoring coursework onto a new PC needs them before SageMath exists. */}
      <section aria-label="Workspaces">
        <div className="section-head" style={{ marginBottom: "var(--sp-3)" }}>
          <h2>
            Workspaces <span className="count">{workspaces.length}</span>
          </h2>
          <div className="btn-row">
            <button className="btn" disabled={busy} onClick={() => setCreating((v) => !v)}>
              <Icon name="add" size={16} />
              New workspace
            </button>
            <button className="btn" disabled={busy} onClick={addFolder}>
              <Icon name="folder" size={16} />
              Add existing folder
            </button>
          </div>
        </div>

        <p className="small workspace-description">
          Optional folders for your courses or projects. Put files in each folder and arrange them
          however you like. Drag files from Windows onto a workspace card to copy them into that
          folder.
        </p>

        {dropped && !dropping && (
          <p className="small" role="status" style={{ marginTop: "var(--sp-2)" }}>
            {dropped.error
              ? dropped.error
              : `${dropped.added} file(s) copied into ${dropped.workspace}.${
                  dropped.failed ? ` ${dropped.failed} couldn't be copied.` : ""
                } Nothing already there was replaced.`}
          </p>
        )}

        {creating && (
          <div className="card" style={{ marginBottom: "var(--sp-3)", maxWidth: "520px" }}>
            <h3>Name your workspace</h3>
            <p className="small" style={{ margin: "var(--sp-1) 0 var(--sp-3)" }}>
              Something like Calculus or Physics. SageDock creates an ordinary Windows folder with
              that name.
            </p>
            <label className="field" htmlFor="new-workspace-name">
              Workspace name
              <input
                id="new-workspace-name"
                className="input"
                value={newName}
                autoFocus
                disabled={busy}
                placeholder="Calculus"
                onChange={(e) => setNewName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") createWorkspace();
                  if (e.key === "Escape") setCreating(false);
                }}
              />
            </label>
            <div className="btn-row" style={{ marginTop: "var(--sp-3)" }}>
              <button
                className="btn btn-accent"
                disabled={busy || !newName.trim()}
                onClick={createWorkspace}
              >
                Create workspace
              </button>
              <button className="btn" disabled={busy} onClick={() => setCreating(false)}>
                Cancel
              </button>
            </div>
          </div>
        )}

        {workspaces.length ? (
          <div className="ws-list">
            {workspaces.map((workspace) => (
              <WorkspaceCard
                key={workspace.id}
                workspace={workspace}
                busy={busy}
                dropping={dropping === workspace.id && !busy}
                launchDisabled={!ready}
                canForget={workspaces.length > 1}
                onLaunch={() => launch(workspace)}
                onRename={(name) => rename(workspace, name)}
                onAddFiles={() => addFiles(workspace)}
                onReveal={() => reveal(workspace)}
                onForget={() => setForgetting(workspace)}
              />
            ))}
          </div>
        ) : (
          <div className="card empty">
            <Icon name="folder" size={24} />
            <h3>No workspaces yet</h3>
            <p>Create a workspace for each course, or add a folder you already have.</p>
          </div>
        )}
      </section>

      {ready && (
        <section aria-label="Recently Opened">
          <div className="section-head" style={{ marginBottom: "var(--sp-3)" }}>
            <h2>
              Recently Opened <span className="count">{recent.length}</span>
            </h2>
            <div className="search-field">
              <Icon name="search" size={16} />
              <input
                className="input"
                aria-label="Find a notebook"
                placeholder="Find a notebook"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>
          </div>

          {visibleRecent.length ? (
            <div className="list">
              {visibleRecent.map((n) => (
                <NotebookRow
                  key={n.path}
                  name={n.name}
                  subtitle={n.path.split("/").slice(0, -1).join(" / ")}
                  modified={n.modified}
                  busy={busy}
                  draggable
                  onDragStart={dragOut(n.path, "workspace")}
                  onOpen={() => open(n.path)}
                  actions={[
                    {
                      key: "delete",
                      label: `Delete ${n.name}`,
                      icon: "remove",
                      danger: true,
                      onClick: () => setDeletingRecent(n),
                    },
                    {
                      key: "reveal",
                      label: `Show ${n.name} in File Explorer`,
                      icon: "folderOpen",
                      onClick: () => revealRecent(n),
                    },
                    {
                      key: "open",
                      label: `Open ${n.name}`,
                      icon: "play",
                      onClick: () => open(n.path),
                    },
                  ]}
                />
              ))}
            </div>
          ) : (
            <div className="card empty">
              <Icon name="document" size={24} />
              <h3>{search ? "No notebooks match that name" : "No notebooks yet"}</h3>
              <p>
                {search ? "Try another name." : "Create a Sage or Python notebook to get started."}
              </p>
            </div>
          )}

          <p className="small" style={{ marginTop: "var(--sp-2)", color: "var(--text-3)" }}>
            Drag a notebook onto your desktop or a folder to save a copy there. The original stays
            in SageDock.
          </p>
        </section>
      )}

      {/* Downloads are not in a workspace yet, so this list is deliberately separate from
          the one above rather than mixed into it. Named plainly "Downloads" because it holds
          every browser's downloads, including the built-in Sage browser's, not only ones
          SageDock itself fetched. */}
      {ready && (
        <section aria-label="Downloads">
          <div className="section-head" style={{ marginBottom: "var(--sp-3)" }}>
            <h2>
              Downloads <span className="count">{downloads.length}</span>
            </h2>
            <div className="btn-row">
              <button
                type="button"
                className="info-tip"
                aria-label="About this list"
                title="Your Windows Downloads folder, plus anything the built-in Sage browser downloaded. Files are listed wherever you chose to save them, and each row says which folder it is in. Includes notebooks a browser saved with a .json name. Read when SageDock opens: choose Refresh after downloading something new."
              >
                <Icon name="info" size={14} />
              </button>
              <button className="btn btn-subtle" disabled={busy} onClick={refreshDownloads}>
                <Icon name="refresh" size={16} />
                Refresh
              </button>
            </div>
          </div>

          {downloads.length ? (
            <div className="list">
              {downloads.map((n) => (
                <NotebookRow
                  key={n.path}
                  name={n.name}
                  // A download saved somewhere other than Downloads says where it went,
                  // so a list spanning several folders stays unambiguous.
                  subtitle={n.folder ?? "Downloads"}
                  modified={n.modified}
                  busy={busy}
                  draggable
                  onDragStart={dragOut(n.path, "downloads")}
                  // Only a notebook has a workspace to be added to. Anything else the
                  // built-in browser downloaded opens in whatever Windows uses for it.
                  onOpen={() =>
                    n.notebook === false ? openDownloadExternally(n) : chooseDownloadWorkspace(n)
                  }
                  actions={[
                    {
                      key: "delete",
                      label: `Delete ${n.name}`,
                      icon: "remove",
                      danger: true,
                      onClick: () => setDeletingDownload(n),
                    },
                    ...(n.notebook === false
                      ? []
                      : [
                          {
                            key: "add",
                            label: `Add ${n.name} to a workspace`,
                            icon: "add" as const,
                            onClick: () => chooseDownloadWorkspace(n),
                          },
                        ]),
                    {
                      key: "reveal",
                      label: `Show ${n.name} in File Explorer`,
                      icon: "folderOpen",
                      onClick: () => revealDownload(n),
                    },
                  ]}
                />
              ))}
            </div>
          ) : (
            <div className="card empty">
              <Icon name="document" size={24} />
              <h3>No downloaded notebooks yet</h3>
              <p>
                Notebooks you download from a browser or email appear here. Anything you download
                from the built-in Sage browser is listed too, wherever you choose to save it.
              </p>
            </div>
          )}
        </section>
      )}

      {/* Which extra tools are available, without ever starting the environment to find
          out. A cached or unknown answer is labelled as such rather than shown as absence. */}
      {status?.environment_installed && (
        <section className="card" aria-label="Compilers and build tools">
          <div className="section-head">
            <h2>Compilers and build tools</h2>
            <div className="btn-row">
              {tools?.source !== "verified" && (
                <button className="btn btn-subtle" disabled={busy} onClick={checkTools}>
                  <Icon name="refresh" size={16} />
                  Check now
                </button>
              )}
              <Link className="btn btn-subtle" to="/tools">
                Manage tools
              </Link>
            </div>
          </div>

          <div className="capability-strip">
            {CAPABILITIES.map((capability) => {
              const found = tools?.tools.find((t) => t.id === capability.id);
              const known = tools?.source !== "unavailable" && !!found;
              const tone = !known
                ? "is-unknown"
                : found.state === "installed"
                  ? "is-ok"
                  : found.state === "needs_repair"
                    ? "is-warn"
                    : "is-off";
              // Never colour alone: every chip carries the state as a word too.
              const word = !known
                ? "Status unknown"
                : found.state === "installed"
                  ? (found.version ?? "Installed")
                  : found.state === "needs_repair"
                    ? "Needs repair"
                    : "Not installed";
              return (
                <span className={`capability ${tone}`} key={capability.id}>
                  <Icon name={capability.icon} size={14} />
                  {capability.name}
                  <small>{word}</small>
                </span>
              );
            })}
          </div>

          <p className="small" style={{ marginTop: "var(--sp-3)", color: "var(--text-3)" }}>
            {tools?.source === "verified" && tools.checked_at
              ? `Checked ${new Date(tools.checked_at).toLocaleString()}.`
              : tools?.source === "cached"
                ? `Last known result${tools.checked_at ? ` from ${new Date(tools.checked_at).toLocaleString()}` : ""}. ${tools.reason ?? ""}`
                : (tools?.reason ??
                  "SageDock hasn't checked which tools are installed on this PC.")}
          </p>

          <p className="small">
            These build packages that need compiling during installation. They do not add new
            notebook languages: your notebooks run SageMath and Python.
          </p>
        </section>
      )}

      <section aria-label="Backups">
        <div className="section-head" style={{ marginBottom: "var(--sp-3)" }}>
          <h2>Backups</h2>
        </div>

        {transfer && task.busy && (
          <div className="progress-block" role="status" style={{ marginTop: 0 }}>
            <div className="progress-top">
              <span className="spinner" />
              {transfer.stage}
            </div>
            <p>{transfer.detail || "This can take a few minutes for a large workspace."}</p>
            <progress aria-label={transfer.stage} max={1} value={transfer.percent ?? undefined} />
          </div>
        )}

        <div className="tool-grid">
          <div className="tool-card">
            <div className="tool-top">
              <span className="tool-icon">
                <Icon name="save" size={16} />
              </span>
              <h3>Create backup</h3>
            </div>
            <p>
              Saves every workspace, including notebooks, datasets, and checkpoints, into one file
              you can keep on a USB drive or in the cloud. Save open notebooks first. SageDock
              checks the backup before reporting success.
            </p>
            <div className="tool-foot">
              <button className="btn" disabled={busy} onClick={backUp}>
                Create backup
              </button>
            </div>
          </div>

          <div className="tool-card">
            <div className="tool-top">
              <span className="tool-icon">
                <Icon name="refresh" size={16} />
              </span>
              <h3>Restore backup</h3>
            </div>
            <p>
              Brings work back from a SageDock backup, including one made on another computer.
              SageDock shows what's inside first and always restores into a separate workspace.
            </p>
            <div className="tool-foot">
              <button className="btn" disabled={busy} onClick={chooseRestore}>
                Restore backup
              </button>
            </div>
          </div>
        </div>
      </section>

      <p className="small mono" style={{ color: "var(--text-3)" }}>
        Notebooks are stored in {status?.workspace_path || "your Windows Documents folder"}
      </p>

      {restart && (
        <ConfirmDialog
          title="Restart Windows?"
          confirm="Restart now"
          onCancel={() => setRestart(false)}
          onConfirm={() => void task.run("Restarting Windows", commands.restartWindows)}
        >
          <p>
            Save your work in all open applications first. After Windows restarts, reopen SageDock
            and choose Continue setup.
          </p>
        </ConfirmDialog>
      )}

      {stopping && (
        <ConfirmDialog
          title="Stop SageMath?"
          confirm="Stop SageMath"
          busy={!!task.busy}
          onCancel={() => setStopping(false)}
          onConfirm={stopEnvironment}
        >
          <p>
            <strong>Save every open notebook first.</strong> Anything you haven't saved may be lost,
            and calculations that are still running will stop.
          </p>
          <p>
            Stopping frees the memory SageMath is using. SageDock stays open, and the next time you
            open a notebook or launch a workspace it starts again on its own.
          </p>
          <p>Files already saved on your computer are kept.</p>
        </ConfirmDialog>
      )}

      {/* Renaming moves the folder, so a notebook open inside it stops the rename. This is
          a pop-up rather than a banner because it answers a question the user has just
          asked, they typed a name and pressed Save, and the answer is actionable. */}
      {renameBlocked && (
        <ConfirmDialog
          title="SageMath is using this workspace"
          confirm="Close"
          dismissOnly
          onCancel={() => setRenameBlocked(null)}
          onConfirm={() => setRenameBlocked(null)}
        >
          <p>{renameBlocked.message}</p>
          <p>
            Renaming <strong>{renameBlocked.name}</strong> moves its folder in Windows, and that
            can't happen while a notebook inside it is open.
          </p>
          <p>
            Save your open notebooks, choose <strong>Stop SageMath</strong> above, then rename it.
          </p>
        </ConfirmDialog>
      )}

      {forgetting && (
        <ConfirmDialog
          title={`Remove ${forgetting.name} from your list?`}
          confirm="Remove from list"
          busy={!!task.busy}
          onCancel={() => setForgetting(null)}
          onConfirm={confirmForget}
        >
          <p>
            <strong>Nothing will be deleted.</strong> This only removes {forgetting.name} from Home.
            The folder and every file inside it stay exactly where they are.
          </p>
          <p className="small mono">{forgetting.path}</p>
          <p>You can add it again later with Add existing folder.</p>
        </ConfirmDialog>
      )}

      {pendingRestore && (
        <ConfirmDialog
          title="Restore this backup?"
          confirm={pendingRestore.preview.compatible ? "Restore backup" : "Close"}
          busy={!!task.busy}
          onCancel={() => setPendingRestore(null)}
          onConfirm={
            pendingRestore.preview.compatible ? confirmRestore : () => setPendingRestore(null)
          }
        >
          <p>
            <strong>{pendingRestore.file_name}</strong>
            <br />
            Made with SageDock {pendingRestore.preview.app_version} ·{" "}
            {pendingRestore.preview.file_count} file(s) ·{" "}
            {formatBytes(pendingRestore.preview.total_bytes)}
          </p>
          {pendingRestore.preview.note ? (
            <p>{pendingRestore.preview.note}</p>
          ) : (
            <>
              <p>
                Your existing workspaces are never overwritten. This backup will be restored as:
              </p>
              <div className="list">
                {pendingRestore.preview.workspaces.map((w) => (
                  <div className="list-row" key={w.name}>
                    <Icon name="folder" size={16} />
                    <span className="list-grow">
                      <strong>
                        {w.restored_as}
                        {w.conflicts ? " (kept separate, you already have one with this name)" : ""}
                      </strong>
                      <small>
                        {w.file_count} file(s) · {formatBytes(w.total_bytes)}
                      </small>
                    </span>
                  </div>
                ))}
              </div>
            </>
          )}
        </ConfirmDialog>
      )}

      {pendingImport && (
        <ChoiceDialog
          title="Add to which workspace?"
          description={
            <p>
              Choose a workspace for <strong>{pendingImport.displayName}</strong>. It will be copied
              in and opened there.
            </p>
          }
          options={workspaceOptions}
          onChoose={pickWorkspaceForImport}
          onCancel={() => setPendingImport(null)}
        />
      )}

      {importConflict && (
        <ChoiceDialog
          title="This notebook already exists"
          description={
            <p>
              <strong>{importConflict.targetName}</strong> already exists in{" "}
              <strong>{importConflict.workspaceName}</strong>. What would you like to do?
            </p>
          }
          options={[
            {
              key: "open_existing",
              label: "Open the existing notebook",
              detail: "Nothing is copied. Opens what's already there.",
              icon: "document",
            },
            {
              key: "copy",
              label: "Make another copy",
              detail: "Adds this one alongside the existing notebook, under a new name.",
              icon: "add",
            },
            {
              key: "replace",
              label: "Replace it",
              detail: "Overwrites the existing notebook with this one. This can't be undone.",
              icon: "warning",
              danger: true,
            },
          ]}
          onChoose={resolveImportConflict}
          onCancel={() => setImportConflict(null)}
        />
      )}

      {deletingRecent && (
        <ConfirmDialog
          title={`Delete ${deletingRecent.name}?`}
          confirm="Move to Recycle Bin"
          busy={!!task.busy}
          onCancel={() => setDeletingRecent(null)}
          onConfirm={confirmDeleteRecent}
        >
          <p>
            <strong>{deletingRecent.name}</strong> will be moved to the Windows Recycle Bin, the
            same as deleting it in File Explorer. You can restore it from there if this was a
            mistake.
          </p>
        </ConfirmDialog>
      )}

      {explaining && <SetupExplainer onClose={() => setExplaining(false)} />}

      {deletingDownload && (
        <ConfirmDialog
          title={`Delete ${deletingDownload.name}?`}
          confirm="Move to Recycle Bin"
          busy={!!task.busy}
          onCancel={() => setDeletingDownload(null)}
          onConfirm={confirmDeleteDownload}
        >
          <p>
            <strong>{deletingDownload.name}</strong> will be moved to the Windows Recycle Bin. This
            only removes it from Downloads. Any copy already added to a workspace is unaffected.
          </p>
        </ConfirmDialog>
      )}
    </>
  );
}
