import { useEffect, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { Link } from "react-router-dom";
import { NavRail } from "./NavRail";
import { ErrorBanner } from "./ErrorBanner";
import { ConfirmDialog } from "./ConfirmDialog";
import { Icon } from "./Icon";
import { useTask } from "../state/TaskContext";
import { useConfig } from "../state/ConfigContext";
import { useSetup } from "../state/SetupContext";
import { commands, isActivePhase } from "../lib/commands";

/** What the app-wide strip says about each phase, and how urgent it looks. */
const SETUP_BANNER = {
  waiting_for_permission: {
    tone: "is-attention",
    text: "Setup needs your permission. Look for the Windows permission window.",
  },
  waiting_for_windows: {
    tone: "is-waiting",
    text: "Setup is waiting for Windows to finish applying changes.",
  },
  running: { tone: "is-working", text: "Setting up SageMath." },
} as const;

export function AppShell({ children }: { children: ReactNode }) {
  const task = useTask();
  const { loadError } = useConfig();
  const { snapshot } = useSetup();
  const [close, setClose] = useState(false);
  const [blocked, setBlocked] = useState(false);

  // Setup is visible from every page, with a way back to it. Previously the only sign of
  // a running setup was the generic busy bar, which was driven by a frontend promise and
  // so could outlive the work, or vanish while the work continued.
  const setupBanner =
    snapshot && isActivePhase(snapshot.phase)
      ? SETUP_BANNER[snapshot.phase as keyof typeof SETUP_BANNER]
      : null;

  // The backend prevents the window closing while work is in flight, and tells us whether
  // it was busy so the dialog can say which situation the user is in.
  useEffect(() => {
    const subscription = listen<boolean>("close-requested", (event) => {
      setBlocked(event.payload);
      setClose(true);
    });
    return () => {
      subscription.then((stop) => stop()).catch(() => {});
    };
  }, []);

  return (
    <div className="app-shell">
      <a href="#main-content" className="skip-link">
        Skip to content
      </a>
      <NavRail />

      <div className="app-main">
        <main id="main-content" className="app-content" tabIndex={-1}>
          <div className="app-content__inner">
            {loadError && <ErrorBanner title="Settings are using defaults" message={loadError} />}

            {setupBanner && (
              <div className={`task-bar setup-banner ${setupBanner.tone}`} role="status">
                <span className="spinner" />
                {setupBanner.text}
                <span className="trailing">
                  <Link to="/" className="btn btn-subtle btn-inline">
                    Show setup
                  </Link>
                </span>
              </div>
            )}

            {task.busy && (
              <div className="task-bar" role="status">
                <span className="spinner" />
                {task.busy}
                <span className="trailing">Keep SageDock open while this finishes.</span>
              </div>
            )}

            {task.error && (
              <ErrorBanner
                title={task.error.title}
                message={task.error.message}
                technicalDetails={task.error.details}
              />
            )}

            {task.message && (
              <div className="notice" role="status">
                <Icon name="success" size={16} />
                {task.message}
                <button onClick={task.dismiss} aria-label="Dismiss message">
                  <Icon name="remove" size={14} />
                </button>
              </div>
            )}

            {children}
          </div>
        </main>
      </div>

      {close && (
        <ConfirmDialog
          title={blocked ? "SageDock is still working" : "Close SageDock?"}
          confirm={blocked ? "Keep SageDock open" : "Stop and close"}
          busy={!!task.busy && !blocked}
          onCancel={() => setClose(false)}
          onConfirm={() => {
            if (blocked) setClose(false);
            else void task.run("Closing SageDock", commands.shutdownAndQuit);
          }}
        >
          <p>
            {blocked
              ? "Let the current task finish before closing. This protects the computing environment from an interrupted change."
              : "Save changes in every open notebook first. Closing stops running calculations and disconnects notebook windows and browser tabs. Files already saved on your computer are kept."}
          </p>
        </ConfirmDialog>
      )}
    </div>
  );
}
