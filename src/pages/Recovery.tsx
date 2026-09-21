import { useState } from "react";
import { Link } from "react-router-dom";
import { commands, type RecoveryId } from "../lib/commands";
import { useTask } from "../state/TaskContext";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { Icon } from "../components/Icon";

/** Ordered least to most invasive, as the product spec requires. */
const ACTIONS: { id: RecoveryId; title: string; copy: string; button: string }[] = [
  {
    id: "verify",
    title: "Check SageMath and Python",
    copy: "Runs a calculation and checks the notebook engines and the secure connection.",
    button: "Run full check",
  },
  {
    id: "service",
    title: "Restart the notebook service",
    copy: "Start here if a notebook won't connect or has stopped responding.",
    button: "Restart notebooks",
  },
  {
    id: "environment",
    title: "Restart the computing environment",
    copy: "Gives SageMath and Python a fresh start when restarting notebooks doesn't help.",
    button: "Restart environment",
  },
  {
    id: "rebuild",
    title: "Reinstall the computing environment",
    copy: "Replaces damaged software with the approved SageMath package. SageDock backs up the previous environment first and restores it if installation fails. Allow at least 30 GB free.",
    button: "Back up & reinstall",
  },
  {
    id: "restore",
    title: "Restore the previous environment",
    copy: "Returns to the backup created before the last reinstall. Your Windows notebooks stay in place.",
    button: "Restore backup",
  },
];

export function Recovery() {
  const task = useTask();
  const [pending, setPending] = useState<(typeof ACTIONS)[number] | null>(null);

  return (
    <>
      <header className="page-head">
        <div>
          <h1>Recovery</h1>
          <p>Start with a check, then work down the list if a repair is needed.</p>
        </div>
      </header>

      <div className="banner banner--warning" role="note">
        <Icon name="info" size={16} />
        <div className="banner__body">
          <p className="banner__title">Your saved notebooks stay with you</p>
          <p className="banner__message">
            Recovery works on the computing software. It never removes notebooks or projects from
            your Windows workspace.
          </p>
        </div>
      </div>

      <section aria-label="Recovery actions">
        <div className="settings-group">
          {ACTIONS.map((action) => (
            <div className="action-row" key={action.id}>
              <div>
                <h3>{action.title}</h3>
                <p>{action.copy}</p>
              </div>
              <button
                className="btn"
                disabled={!!task.busy}
                onClick={() =>
                  action.id === "verify"
                    ? void task.run("Checking SageMath and Python", () =>
                        commands.recover(action.id),
                      )
                    : setPending(action)
                }
              >
                {action.button}
              </button>
            </div>
          ))}
        </div>
      </section>

      <div className="btn-row">
        <button
          className="btn btn-subtle"
          disabled={!!task.busy}
          onClick={() =>
            void task.run("Choosing SageMath package", async () => {
              await commands.chooseSagePackage();
            })
          }
        >
          <Icon name="openFile" size={16} />
          Choose replacement package
        </button>
        <Link className="btn btn-subtle" to="/diagnostics">
          <Icon name="diagnostic" size={16} />
          Open diagnostics
        </Link>
      </div>

      {pending && (
        <ConfirmDialog
          title={pending.title}
          confirm={pending.button}
          busy={!!task.busy}
          onCancel={() => setPending(null)}
          onConfirm={() => {
            const action = pending.id;
            setPending(null);
            void task.run(pending.title, () => commands.recover(action));
          }}
        >
          <p>
            <strong>Save changes in every notebook before continuing.</strong> This closes notebook
            windows and stops running calculations. Browser tabs will disconnect.
          </p>
          <p>{pending.copy}</p>
          <p>
            <strong>Files already saved in your Windows workspace are preserved.</strong>
          </p>
        </ConfirmDialog>
      )}
    </>
  );
}
