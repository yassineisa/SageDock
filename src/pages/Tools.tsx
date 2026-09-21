import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { listen } from "@tauri-apps/api/event";
import {
  commands,
  friendlyError,
  type ToolProgress,
  type ToolReport,
  type ToolStatus,
  type ToolId,
} from "../lib/commands";
import { useTask } from "../state/TaskContext";
import { ErrorBanner } from "../components/ErrorBanner";
import { Icon, type IconName } from "../components/Icon";

/**
 * Plain-language descriptions. Deliberately say what the tool is *for*, because a student
 * who knows they need "a Fortran compiler" does not need this page, and one who does not
 * know cannot act on the word "gfortran".
 *
 * Note what none of these claim: installing a compiler does **not** add a C, C++, or
 * Fortran notebook language. SageDock's notebooks run SageMath and Python; these compilers
 * exist so that packages which build from source can be built.
 */
const CATALOGUE: {
  id: ToolId;
  name: string;
  copy: string;
  group: string;
  icon: IconName;
}[] = [
  {
    id: "cpp",
    name: "C / C++ compiler",
    copy: "Builds Python and Sage packages that are written in C or C++ and have no ready-made download.",
    group: "Compilers and build tools",
    icon: "code",
  },
  {
    id: "fortran",
    name: "Fortran compiler",
    copy: "Needed by some numerical and scientific packages, which still use Fortran for their fastest routines.",
    group: "Compilers and build tools",
    icon: "numeric",
  },
  {
    id: "build",
    name: "Build tools",
    copy: "Make, CMake, and pkg-config. Packages use these to work out how to build themselves.",
    group: "Compilers and build tools",
    icon: "buildTools",
  },
  {
    id: "toolkit",
    name: "Full scientific build toolkit",
    copy: "Everything above in one step. Recommended if a course asks you to compile packages from source. Working tools are checked first; missing or damaged packages are repaired.",
    group: "Compilers and build tools",
    icon: "toolkit",
  },
  {
    id: "seaborn",
    name: "Seaborn",
    copy: "Statistical charts built on matplotlib.",
    group: "Data and analysis",
    icon: "calculator",
  },
  {
    id: "statsmodels",
    name: "Statsmodels",
    copy: "Statistical models, regression, and time series.",
    group: "Data and analysis",
    icon: "calculator",
  },
  {
    id: "polars",
    name: "Polars",
    copy: "Fast tables for organising and transforming data.",
    group: "Data and analysis",
    icon: "calculator",
  },
];

/** The seven states the page can show, as one value so the UI cannot contradict itself. */
type Presented =
  | "checking"
  | "not_installed"
  | "installing"
  | "verifying"
  | "installed"
  | "needs_repair"
  | "unavailable";

const LABEL: Record<Presented, string> = {
  checking: "Checking",
  not_installed: "Not installed",
  installing: "Installing",
  verifying: "Verifying",
  installed: "Installed",
  needs_repair: "Needs repair",
  unavailable: "Status unavailable",
};

function presentedState(
  report: ToolReport | null,
  status: ToolStatus | undefined,
  active: ToolId | null,
  id: ToolId,
  stage: ToolProgress["stage"] | null,
): Presented {
  if (active === id) {
    if (stage === "verifying") return "verifying";
    if (stage === "checking" || !stage) return "checking";
    return "installing";
  }
  if (!report) return "checking";
  // An unknown status is never reported as "not installed": those mean different things.
  if (report.source === "unavailable" || !status) return "unavailable";
  if (status.state === "installed") return "installed";
  if (status.state === "needs_repair") return "needs_repair";
  return "not_installed";
}

function toneOf(state: Presented): string {
  if (state === "installed") return "is-ok";
  if (state === "needs_repair") return "is-warn";
  if (state === "unavailable" || state === "checking") return "is-unknown";
  if (state === "installing" || state === "verifying") return "is-unknown";
  return "is-off";
}

function iconFor(state: Presented): IconName {
  if (state === "installed") return "success";
  if (state === "needs_repair") return "warning";
  if (state === "unavailable") return "info";
  return "add";
}

export function Tools() {
  const task = useTask();
  const [report, setReport] = useState<ToolReport | null>(null);
  const [error, setError] = useState<ReturnType<typeof friendlyError> | null>(null);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState<ToolId | null>(null);
  const [progress, setProgress] = useState<ToolProgress | null>(null);

  async function refresh(force: boolean) {
    try {
      setReport(await commands.scientificTools(force));
      setError(null);
    } catch (e) {
      setReport({
        source: "unavailable",
        checked_at: null,
        reason: "The tool check failed. Retry or open Recovery to check the environment.",
        tools: [],
      });
      setError(friendlyError(e));
    }
  }

  useEffect(() => {
    if (!task.busy) void refresh(false);
  }, [task.busy]);

  useEffect(() => {
    const sub = listen<ToolProgress>("tool-progress", (e) => setProgress(e.payload));
    return () => {
      sub.then((f) => f()).catch(() => {});
    };
  }, []);

  const install = (tool: (typeof CATALOGUE)[number], repairing: boolean) => {
    setActive(tool.id);
    setProgress(null);
    void task
      .run(`${repairing ? "Repairing" : "Installing"} ${tool.name}`, async () => {
        const next = await commands.installScientificTool(tool.id);
        setReport(next);
        return `${tool.name} is installed and passed its check. Restart the session in an open notebook before using it.`;
      })
      .finally(() => {
        setActive(null);
        setProgress(null);
      });
  };

  const checkNow = () =>
    void task.run("Checking installed tools", async () => {
      try {
        setReport(await commands.scientificTools(true));
        setError(null);
      } catch (e) {
        setReport({
          source: "unavailable",
          checked_at: null,
          reason: "The tool check failed. Retry or open Recovery to check the environment.",
          tools: [],
        });
        throw e;
      }
    });

  const matches = CATALOGUE.filter((t) =>
    (t.name + " " + t.copy).toLowerCase().includes(query.toLowerCase()),
  );
  const groups = [...new Set(matches.map((t) => t.group))];
  const checkedAt = report?.checked_at ? new Date(report.checked_at).toLocaleString() : null;

  return (
    <>
      <header className="page-head">
        <div>
          <h1>Scientific tools</h1>
          <p>
            Add compilers and Python packages your courses need after setting up SageMath from Home.
          </p>
        </div>
        <div className="search-field">
          <Icon name="search" size={16} />
          <input
            className="input"
            aria-label="Find a scientific tool"
            placeholder="Find a tool"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
      </header>

      {error && (
        <div>
          <ErrorBanner
            title={error.title}
            message={error.message}
            technicalDetails={error.details}
          />
          <div className="btn-row" style={{ marginTop: "var(--sp-3)" }}>
            <Link className="btn" to="/">
              Finish setup
            </Link>
          </div>
        </div>
      )}

      {/* Where the numbers came from is part of the answer. A cached or unknown status is
          never presented as though SageDock had just looked and found nothing. */}
      {report && report.source !== "verified" && (
        <div className="banner banner--warning" role="note">
          <Icon name="info" size={16} />
          <div className="banner__body">
            <p className="banner__title">
              {report.source === "cached"
                ? "Showing the last known result"
                : "SageDock hasn't checked these yet"}
            </p>
            <p className="banner__message">
              {report.reason}
              {report.source === "cached" && checkedAt ? ` Last checked ${checkedAt}.` : ""}
            </p>
          </div>
        </div>
      )}

      <div className="btn-row">
        <button className="btn" disabled={!!task.busy} onClick={checkNow}>
          <Icon name="refresh" size={16} />
          Check now
        </button>
        {report?.source === "verified" && checkedAt && (
          <span className="small" style={{ color: "var(--text-3)" }}>
            Checked {checkedAt}
          </span>
        )}
      </div>

      {groups.map((group) => (
        <section key={group} aria-label={group}>
          <span className="section-label">{group}</span>
          <div className="tool-grid">
            {matches
              .filter((t) => t.group === group)
              .map((tool) => {
                const status = report?.tools.find((r) => r.id === tool.id);
                const state = presentedState(
                  report,
                  status,
                  active,
                  tool.id,
                  progress?.stage ?? null,
                );
                const busyHere = active === tool.id;
                return (
                  <article className="tool-card" key={tool.id}>
                    <div className="tool-top">
                      <span className="tool-icon">
                        <Icon name={tool.icon} size={16} />
                      </span>
                      <h3>{tool.name}</h3>
                    </div>
                    <p>{tool.copy}</p>

                    <div className={`tool-state ${toneOf(state)}`}>
                      {busyHere ? (
                        <span className="spinner" />
                      ) : (
                        <Icon name={iconFor(state)} size={14} />
                      )}
                      {LABEL[state]}
                      {status?.version && state === "installed" && (
                        <span
                          className="small mono"
                          style={{ fontWeight: 400, marginLeft: "auto" }}
                        >
                          {status.version}
                        </span>
                      )}
                    </div>

                    {busyHere && progress && (
                      <div className="progress-block" role="status" style={{ margin: 0 }}>
                        <div className="progress-top">
                          <span className="spinner" />
                          {progress.title}
                        </div>
                        {progress.detail && <p>{progress.detail}</p>}
                        <progress
                          aria-label={progress.title}
                          max={1}
                          value={progress.percent ?? undefined}
                        />
                      </div>
                    )}

                    {/* Each program is listed separately, because "build tools" is three
                        independent programs and a student whose Make is missing needs to be
                        told that rather than shown one misleading tick. */}
                    {!!status?.components.length && !busyHere && (
                      <div className="tool-components">
                        {status.components.map((component) => (
                          <div
                            className={`component-row ${component.works ? "is-ok" : "is-off"}`}
                            key={component.id}
                          >
                            <Icon name={component.works ? "success" : "remove"} size={12} />
                            <span className="component-name">{component.label}</span>
                            <span className="component-version mono">
                              {component.works
                                ? (component.version ?? "Working")
                                : component.present
                                  ? "Not working"
                                  : "Missing"}
                            </span>
                          </div>
                        ))}
                      </div>
                    )}

                    <div className="tool-foot">
                      {state === "installed" ? (
                        <button className="btn" disabled={!!task.busy} onClick={checkNow}>
                          Verify
                        </button>
                      ) : state === "needs_repair" ? (
                        <button
                          className="btn btn-accent"
                          disabled={!!task.busy}
                          onClick={() => install(tool, true)}
                        >
                          Repair
                        </button>
                      ) : state === "unavailable" || state === "checking" ? (
                        <button className="btn" disabled={!!task.busy} onClick={checkNow}>
                          Check now
                        </button>
                      ) : (
                        <button
                          className="btn"
                          disabled={!!task.busy}
                          onClick={() => install(tool, false)}
                        >
                          {tool.id === "toolkit" ? "Install all" : "Install"}
                        </button>
                      )}
                    </div>
                  </article>
                );
              })}
          </div>
        </section>
      ))}

      {!matches.length && (
        <div className="card empty">
          <Icon name="search" size={24} />
          <h3>No tools match that search</h3>
          <p>Try the name of a compiler or a Python package.</p>
        </div>
      )}

      <div className="banner banner--warning" role="note">
        <Icon name="info" size={16} />
        <div className="banner__body">
          <p className="banner__title">Compilers are not notebook languages</p>
          <p className="banner__message">
            These build packages that need compiling during installation. Your notebooks still run
            SageMath and Python. Installing a compiler does not add a C, C++, or Fortran notebook.
          </p>
        </div>
      </div>

      <p className="small">
        Installing a tool needs an internet connection and about 3 GB of free storage. Compilers run
        small test programs; Python packages are checked by importing them. An installation cannot
        be cancelled part way through, because stopping the software manager mid change is what
        leaves it broken. Optional packages are kept separate from SageMath's bundled libraries.
      </p>
    </>
  );
}
