import { useState } from "react";
import { commands } from "../lib/commands";
import { useTask } from "../state/TaskContext";
import { Icon } from "../components/Icon";

type Report = {
  version: string;
  checked_at: string;
  runtime: string;
  checks: { check: string; severity: string; summary: string }[];
};

export function Diagnostics() {
  const task = useTask();
  const [report, setReport] = useState<string | null>(null);
  const parsed = report ? (JSON.parse(report) as Report) : null;

  return (
    <>
      <header className="page-head">
        <div>
          <h1>Diagnostics</h1>
          <p>Check Windows and SageMath, then save a report if you need help.</p>
        </div>
      </header>

      <div className="command-bar">
        <button
          className="btn btn-accent"
          disabled={!!task.busy}
          onClick={() =>
            void task.run("Checking your computer", async () => {
              setReport(await commands.diagnosticReport());
            })
          }
        >
          <Icon name="diagnostic" size={16} />
          {task.busy ? "Checking…" : "Run diagnostics"}
        </button>
        <button
          className="btn"
          disabled={!!task.busy || !report}
          onClick={() =>
            void task.run("Copying report", async () => {
              await navigator.clipboard.writeText(report!);
              return "Diagnostic report copied.";
            })
          }
        >
          Copy report
        </button>
        <button
          className="btn"
          disabled={!!task.busy}
          onClick={() =>
            void task.run("Saving report", async () => {
              const saved = await commands.saveDiagnosticReport();
              if (saved) return "Diagnostic report saved.";
            })
          }
        >
          <Icon name="save" size={16} />
          Save report
        </button>
      </div>

      {parsed ? (
        <section className="card" aria-label="Results">
          <div className="section-head" style={{ marginBottom: "var(--sp-3)" }}>
            <h2>Workspace health</h2>
            <span className="small mono" style={{ color: "var(--text-3)" }}>
              SageDock {parsed.version}
            </span>
          </div>

          {parsed.checks.map((c) => (
            <div
              className={c.severity === "info" ? "check-row check-ok" : "check-row check-warn"}
              key={c.check}
            >
              <Icon name={c.severity === "info" ? "success" : "warning"} size={16} />
              <div>
                <strong>{c.check.split("_").join(" ")}</strong>
                <p>{c.summary}</p>
              </div>
            </div>
          ))}

          <div
            className={parsed.runtime === "healthy" ? "check-row check-ok" : "check-row check-warn"}
          >
            <Icon name={parsed.runtime === "healthy" ? "success" : "warning"} size={16} />
            <div>
              <strong>SageMath and notebook engines</strong>
              <p>
                {parsed.runtime === "healthy"
                  ? "Both notebook engines and the workspace passed their launch checks."
                  : "The computing environment needs setup or recovery."}
              </p>
            </div>
          </div>

          <details style={{ marginTop: "var(--sp-4)" }}>
            <summary>Report details</summary>
            <pre style={{ marginTop: "var(--sp-2)" }}>{report}</pre>
          </details>
        </section>
      ) : (
        <div className="card empty">
          <Icon name="diagnostic" size={24} />
          <h3>No results yet</h3>
          <p>Run a check to see what is working and what needs attention.</p>
        </div>
      )}

      <div className="banner banner--warning" role="note">
        <Icon name="info" size={16} />
        <div className="banner__body">
          <p className="banner__title">Reports keep your work private</p>
          <p className="banner__message">
            Reports exclude notebook contents, filenames, personal paths, and authentication tokens.
            You choose whether to share them.
          </p>
        </div>
      </div>
    </>
  );
}
