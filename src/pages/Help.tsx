import { useState } from "react";
import { Link } from "react-router-dom";
import { Icon } from "../components/Icon";

const GUIDES = [
  {
    title: "Your first Sage notebook",
    tag: "Getting started",
    text: "Choose New Sage Notebook on the Workspace page. Type factor(123456) into a cell and press Shift + Enter, or use the Run button. SageMath should show 2^6 * 3 * 643. Use Sage for exact mathematics, algebra, calculus, and number theory.",
  },
  {
    title: "Your first Python notebook",
    tag: "Getting started",
    text: "Choose New Python Notebook. Try print(2 + 2), then run the cell. NumPy, SciPy, pandas, SymPy, matplotlib, and scikit-learn come with SageDock. Add code and notes together to record your work.",
  },
  {
    title: "Where your work is saved",
    tag: "Your notebooks",
    text: "Your notebooks live in the SageDock folder inside your Windows Documents folder. Use Show in File Explorer to find them. Jupyter saves periodically, but always use File then Save Notebook, or Ctrl + S, before closing SageDock, restarting, or repairing. Unsaved browser edits and running calculations are not preserved by a runtime backup.",
  },
  {
    title: "Open a notebook or project",
    tag: "Your notebooks",
    text: "Choose Open JupyterLab on Home to browse files and start working without creating a notebook first. It opens your current folder; workspaces are optional folders for different courses. New and imported notebooks go directly into that folder, and you can arrange subfolders yourself. Use Open notebook to import an .ipynb file from elsewhere. The original stays untouched and existing names get a numbered copy.",
  },
  {
    title: "Setting up a new computer",
    tag: "Setup and recovery",
    text: "Open SageDock and choose Set up SageDock. Allow at least 15 GB free for the computing environment. Windows may request administrator permission and an internet connection to install a required component. If asked to restart, save your other work, restart Windows, reopen SageDock, and choose Continue setup. SageDock then installs its included SageMath package and tests real calculations before reporting success.",
  },
  {
    title: "When virtualization is turned off",
    tag: "Setup and recovery",
    text: "Some computers have the feature needed to run SageMath switched off in their startup settings. Check your PC manufacturer's instructions for enabling Intel Virtualization Technology or AMD SVM. A school-managed PC may require your IT team. SageDock cannot change firmware settings itself. After enabling the feature, reopen SageDock and run setup again.",
  },
  {
    title: "When a notebook stops responding",
    tag: "Setup and recovery",
    text: "First save any notebook that still responds. In Recovery, run a full check, then restart the notebook service. If needed, restart the computing environment. Reinstall is the last step: SageDock requires an approved package, checks free space, and keeps a full backup of the previous environment. Restore backup can recover an interrupted replacement.",
  },
  {
    title: "Add scientific tools",
    tag: "Scientific tools",
    text: "Open Scientific tools to install a compiler or a supported Python package. Downloads need an internet connection. SageDock verifies the installation before showing Installed. Restart the session from JupyterLab's Kernel menu before using a newly installed Python package.",
  },
  {
    title: "Back up and restore your coursework",
    tag: "Your notebooks",
    text: "Create backup on the Workspace page saves every workspace into a single file, including notebooks, datasets, and checkpoints. Save open notebooks first. Restore backup shows what is inside before changing anything and always restores into a separate workspace, so existing coursework is never overwritten. A backup made on one computer restores on another.",
  },
  {
    title: "Updates and removing SageDock",
    tag: "Maintenance",
    text: "Install updates only from your trusted SageDock distributor. This release does not download app updates automatically. Runtime packages must match an approved release checksum. Removing the Windows app keeps your notebook folders and computing environment.",
  },
];

export function Help() {
  const [query, setQuery] = useState("");

  const matches = GUIDES.filter((g) =>
    (g.title + " " + g.text).toLowerCase().includes(query.toLowerCase()),
  );

  return (
    <>
      <header className="page-head">
        <div>
          <h1>Guides &amp; help</h1>
          <p>Short answers to the questions that come up most.</p>
        </div>
        <div className="search-field">
          <Icon name="search" size={16} />
          <input
            className="input"
            aria-label="Search guides"
            placeholder="Search guides"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
      </header>

      <section aria-label="Guides">
        {matches.map((g) => (
          <details className="guide" key={g.title}>
            <summary>
              <span>
                <span className="guide-tag">{g.tag}</span>
                {g.title}
              </span>
              <Icon name="chevronRight" size={16} />
            </summary>
            <p>{g.text}</p>
          </details>
        ))}
        {!matches.length && (
          <div className="card empty">
            <Icon name="search" size={24} />
            <h3>No guide matches that search</h3>
            <p>Try "save", "setup", "Python", or "repair".</p>
          </div>
        )}
      </section>

      <section className="card" aria-label="More help">
        <h2>Still need help?</h2>
        <p style={{ marginTop: "var(--sp-1)" }}>
          Run diagnostics to check your computer and create a report for your instructor or support
          contact.
        </p>
        <div className="btn-row" style={{ marginTop: "var(--sp-3)" }}>
          <Link className="btn" to="/diagnostics">
            <Icon name="diagnostic" size={16} />
            Open diagnostics
          </Link>
          <Link className="btn btn-subtle" to="/about">
            <Icon name="info" size={16} />
            About SageDock
          </Link>
        </div>
      </section>
    </>
  );
}
