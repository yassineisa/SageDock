import { useEffect, useState } from "react";
import { Icon, type IconName } from "./Icon";
import { commands, type SetupSnapshot, type SetupStep, type StepState } from "../lib/commands";

/**
 * How long a stage may go without a *meaningful* update before the panel says so.
 *
 * Deliberately generous. Enabling Windows components and unpacking the runtime are both
 * genuinely quiet for minutes at a time, and crying "stuck" at a healthy install would
 * teach students to distrust the one message that should mean something.
 */
const SLOW_AFTER_MS = 180_000;

/** Re-render cadence for elapsed time. One second is enough for a clock read in seconds. */
const TICK_MS = 1000;

function formatDuration(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  if (minutes === 0) return `${seconds}s`;
  return `${minutes}m ${String(seconds).padStart(2, "0")}s`;
}

const STEP_ICON: Record<StepState, IconName> = {
  done: "success",
  active: "play",
  failed: "error",
  skipped: "accept",
  pending: "chevronRight",
};

/** Never colour alone: every step carries its state as a word for screen readers too. */
const STEP_WORD: Record<StepState, string> = {
  done: "Done",
  active: "In progress",
  failed: "Failed",
  skipped: "Not needed on this PC",
  pending: "Waiting",
};

function StepRow({ step }: { step: SetupStep }) {
  return (
    <li className={`setup-step is-${step.state}`}>
      <Icon name={STEP_ICON[step.state]} size={16} />
      <span className="setup-step-text">
        <strong>{step.title}</strong>
        <small>{step.state === "active" ? step.explanation : STEP_WORD[step.state]}</small>
      </span>
    </li>
  );
}

/**
 * The live setup view, usable on any screen.
 *
 * Everything here is read from the backend snapshot. Nothing is inferred from how long a
 * promise has been pending, and nothing is invented to fill a gap, an unmeasurable stage
 * shows an indeterminate bar with a real explanation rather than a fabricated percentage.
 */
export function SetupProgress({
  snapshot,
  onExplain,
}: {
  snapshot: SetupSnapshot;
  /** Opens the full explanation. Omitted where the explanation is already on screen. */
  onExplain?: () => void;
}) {
  const [now, setNow] = useState(() => Date.now());
  const [details, setDetails] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);

  const active =
    snapshot.phase === "running" ||
    snapshot.phase === "waiting_for_permission" ||
    snapshot.phase === "waiting_for_windows";

  // The clock only runs while something is running: a finished run has a duration, and
  // re-rendering a static screen every second is waste.
  useEffect(() => {
    if (!active) return;
    const timer = setInterval(() => setNow(Date.now()), TICK_MS);
    return () => clearInterval(timer);
  }, [active]);

  const elapsed = snapshot.started_at ? now - snapshot.started_at : 0;
  const sinceProgress = snapshot.updated_at ? now - snapshot.updated_at : 0;
  const slow = active && sinceProgress > SLOW_AFTER_MS;

  // Who is holding things up. This is the distinction the old UI could not make: one
  // spinner covered the app working, Windows working, and a permission prompt nobody had
  // noticed, which is why an unanswered prompt looked like a freeze.
  const waiting =
    snapshot.phase === "waiting_for_permission"
      ? {
          tone: "is-attention" as const,
          label: "Waiting for you",
          icon: "warning" as IconName,
        }
      : snapshot.phase === "waiting_for_windows"
        ? { tone: "is-waiting" as const, label: "Waiting for Windows", icon: "info" as IconName }
        : { tone: "is-working" as const, label: "SageDock is working", icon: "play" as IconName };

  const exportDiagnostics = async () => {
    try {
      const report = await commands.setupDiagnostics();
      await navigator.clipboard.writeText(report);
      setCopied("Diagnostic report copied. You can paste it into an email or a message.");
    } catch {
      setCopied("SageDock couldn't copy the report to the clipboard.");
    }
  };

  return (
    <section className="setup-progress card" aria-label="Setup progress">
      <div className={`setup-progress-head ${waiting.tone}`}>
        <span className="setup-progress-badge">
          <Icon name={waiting.icon} size={14} />
          {waiting.label}
        </span>
        {active && (
          <span className="small setup-elapsed">
            Running for {formatDuration(elapsed)}
            {sinceProgress > 15_000 && <> · last update {formatDuration(sinceProgress)} ago</>}
          </span>
        )}
      </div>

      <h3>{snapshot.title}</h3>
      {snapshot.detail && <p className="setup-progress-detail">{snapshot.detail}</p>}

      {active && (
        <progress
          aria-label={snapshot.title}
          max={1}
          // Undefined renders the indeterminate bar. The backend sends a number only when
          // a stage can actually measure itself.
          value={snapshot.percent ?? undefined}
        />
      )}

      {/* A heartbeat proves the backend thread is alive and nothing more. Saying so out
          loud is the point: presenting liveness as progress is exactly the false comfort
          that made the original stall so hard to notice. */}
      {active && snapshot.heartbeat_at > snapshot.updated_at && (
        <p className="small setup-heartbeat">
          SageDock is still connected to this task. That confirms it hasn't crashed. It isn't a sign
          that the step is advancing.
        </p>
      )}

      {slow && (
        <div className="setup-slow" role="status">
          <Icon name="info" size={16} />
          <div>
            <strong>This step is taking longer than usual.</strong>
            <p className="small">
              {snapshot.phase === "waiting_for_permission"
                ? "Windows is still waiting for an answer to its permission window. It may be behind SageDock or flashing in the taskbar."
                : "Nothing has gone wrong that SageDock can detect, and the work is still running. Leave it going, or export a diagnostic report if you want to check with someone."}
            </p>
          </div>
        </div>
      )}

      <ol className="setup-steps">
        {snapshot.steps.map((step) => (
          <StepRow key={step.stage} step={step} />
        ))}
      </ol>

      <div className="btn-row setup-progress-actions">
        {onExplain && (
          <button type="button" className="btn btn-subtle" onClick={onExplain}>
            <Icon name="info" size={16} />
            What happens during setup?
          </button>
        )}
        <button
          type="button"
          className="btn btn-subtle"
          aria-expanded={details}
          onClick={() => setDetails((v) => !v)}
        >
          <Icon name="diagnostic" size={16} />
          {details ? "Hide technical details" : "Show technical details"}
        </button>
        <button type="button" className="btn btn-subtle" onClick={() => void exportDiagnostics()}>
          <Icon name="save" size={16} />
          Export diagnostics
        </button>
      </div>

      {copied && (
        <p className="small" role="status">
          {copied}
        </p>
      )}

      {details && (
        <div className="setup-log">
          {snapshot.log.length ? (
            <ol>
              {snapshot.log.map((line, index) => (
                <li key={`${line.at}-${index}`} className="small mono">
                  +{formatDuration(line.at - snapshot.started_at)} {line.text}
                </li>
              ))}
            </ol>
          ) : (
            <p className="small">Nothing has been logged for this run yet.</p>
          )}
        </div>
      )}
    </section>
  );
}
