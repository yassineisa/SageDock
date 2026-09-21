import { useEffect, useRef } from "react";
import { Icon } from "./Icon";
import { useSetup } from "../state/SetupContext";
import type { SetupStage } from "../lib/commands";

/**
 * What "What happens during setup?" opens.
 *
 * It used to link to `/help`, the general questions page, which answered none of the
 * question it was attached to and — worse — navigated away from Home mid-setup, which was
 * the surest way to hit the state-loss bug. This is a modal instead: it explains *this*
 * installation, highlights the step actually running, and closing it returns the user
 * exactly where they were without setup noticing.
 *
 * The stage list comes from the backend snapshot rather than being duplicated here, so the
 * explanation and the progress view cannot disagree about what setup does.
 */
export function SetupExplainer({ onClose }: { onClose: () => void }) {
  const { snapshot } = useSetup();
  const ref = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = ref.current;
    dialog?.showModal();
    return () => dialog?.close();
  }, []);

  const current: SetupStage | null = snapshot?.stage ?? null;
  const running =
    snapshot?.phase === "running" ||
    snapshot?.phase === "waiting_for_permission" ||
    snapshot?.phase === "waiting_for_windows";

  // Falls back to the plan the backend reports even before a run starts, so the
  // explanation is complete on a machine where setup has never been attempted.
  const steps = snapshot?.steps ?? [];

  return (
    <dialog
      ref={ref}
      className="dialog setup-explainer"
      aria-labelledby="setup-explainer-title"
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
    >
      <h2 id="setup-explainer-title">What happens during setup</h2>
      <div className="dialog-body">
        <p>
          SageDock installs SageMath, Python, and Jupyter on this PC and checks that they can
          actually run a notebook. It only has to happen once.
        </p>

        {running && (
          <p className="setup-explainer-live" role="status">
            <Icon name="play" size={14} />
            Setup is running now — the highlighted step below is the one in progress.
          </p>
        )}

        <ol className="setup-steps setup-steps-explained">
          {steps.map((step) => (
            <li
              key={step.stage}
              className={`setup-step is-${step.state}${step.stage === current ? " is-current" : ""}`}
            >
              <Icon
                name={
                  step.state === "done"
                    ? "success"
                    : step.state === "failed"
                      ? "error"
                      : step.stage === current
                        ? "play"
                        : "chevronRight"
                }
                size={16}
              />
              <span className="setup-step-text">
                <strong>{step.title}</strong>
                <small>{step.explanation}</small>
              </span>
            </li>
          ))}
        </ol>

        <h3>Administrator permission</h3>
        <p>
          One step needs permission from Windows, because switching on a Windows feature changes the
          computer rather than just SageDock's own files. Windows shows its own permission window;
          SageDock never sees your password. Nothing else in setup runs with administrator rights,
          and SageDock itself does not.
        </p>
        <p className="small">
          If nothing seems to be happening after you start setup, look for that permission window —
          it can open behind SageDock or flash in the taskbar. Setup cannot continue until you
          answer it.
        </p>

        <h3>A restart may be needed</h3>
        <p>
          Windows sometimes needs to restart before a feature it just enabled will work. If that
          happens, SageDock saves its progress first and tells you. After restarting, reopen
          SageDock and choose <strong>Continue setup</strong> — it picks up from the last finished
          step instead of starting over.
        </p>

        <h3>Where your notebooks are kept</h3>
        <p>
          Your work is stored in an ordinary Windows folder, outside the SageMath environment. That
          separation is deliberate: repairing, reinstalling, or removing SageMath never touches your
          notebooks, and you can open, copy, or back them up in File Explorer like any other file.
        </p>

        <h3>If setup is interrupted</h3>
        <p>
          Closing SageDock, a crash, or a power cut during setup will not damage anything you have
          saved. Each step checks what is already done before it runs, so starting setup again
          continues from where it stopped rather than repeating finished work. SageDock notices an
          interrupted run the next time it opens and says so.
        </p>
        <p className="small">
          Setup only reports success after SageMath and Python have each run a test notebook cell
          for real. A step that cannot be verified is reported as a problem, not quietly passed.
        </p>
      </div>
      <div className="dialog-actions">
        <button className="btn btn-accent" onClick={onClose} autoFocus>
          Close
        </button>
      </div>
    </dialog>
  );
}
