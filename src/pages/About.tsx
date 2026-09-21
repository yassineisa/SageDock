import { useEffect, useState } from "react";
import { commands, type AppInfo } from "../lib/commands";
import { Icon } from "../components/Icon";

// Attribution shown in About. Kept in step with THIRD-PARTY-NOTICES.md, which carries the
// authoritative wording; the definitive license text for each package ships inside the
// runtime image itself. Names use their projects' own casing.
const CREDITS = [
  { name: "SageMath", license: "GPL-2.0-or-later" },
  { name: "Python", license: "Python Software Foundation License" },
  { name: "JupyterLab, Jupyter Server, IPython", license: "BSD-3-Clause" },
  { name: "NumPy, SciPy, pandas, scikit-learn, matplotlib", license: "BSD-3-Clause" },
  { name: "SymPy", license: "BSD-3-Clause" },
  { name: "Ubuntu base system", license: "GPL, LGPL, MIT, BSD, and others" },
  { name: "conda-forge packages", license: "Various, per package" },
  { name: "Tauri", license: "MIT or Apache-2.0" },
  { name: "React, React Router", license: "MIT" },
  { name: "Vite, TypeScript", license: "MIT or Apache-2.0" },
  { name: "Other Rust crates and npm packages", license: "Mostly MIT or Apache-2.0" },
  { name: "Segoe Fluent Icons", license: "Included with Windows, © Microsoft" },
];

/**
 * The single place attribution appears in the interface.
 *
 * The version comes from the running desktop binary rather than a build-time constant, so
 * it stays accurate after an update. There is deliberately no global footer: attribution
 * belongs on one page a user can go to, not on every screen.
 */
export function About() {
  const [about, setAbout] = useState<AppInfo | null>(null);
  const [linkError, setLinkError] = useState(false);

  useEffect(() => {
    commands
      .getAppInfo()
      .then(setAbout)
      .catch(() => {});
  }, []);

  const developer = about?.developer ?? "Yassin Eisa";

  // The address is also the button's label, so a failure still leaves it readable on screen.
  const openProject = () => {
    setLinkError(false);
    commands.openProjectPage().catch(() => setLinkError(true));
  };

  return (
    <>
      <header className="page-head">
        <div>
          <h1>About</h1>
          <p>Version, licensing, and the software SageDock is built on.</p>
        </div>
      </header>

      <section className="card" aria-labelledby="about-heading">
        <span className="section-label">SageDock</span>
        <h2 id="about-heading">SageDock</h2>

        <dl className="about-grid">
          <div>
            <dt>Version</dt>
            <dd className="mono">{about?.version ?? "Unknown"}</dd>
          </div>
          <div>
            <dt>Developer</dt>
            <dd>{developer}</dd>
          </div>
          <div>
            <dt>License</dt>
            <dd>{about?.license ?? "MIT"}</dd>
          </div>
        </dl>

        <p>
          <strong>Created by {developer}.</strong> SageDock's own code is released under the MIT
          License, copyright © 2026 {developer}. The MIT License covers SageDock itself and nothing
          else.
        </p>

        <div className="btn-row" style={{ marginTop: "var(--sp-3)" }}>
          <button className="btn" onClick={openProject}>
            github.com/yassineisa
          </button>
        </div>
        {linkError && (
          <p className="small" style={{ marginTop: "var(--sp-2)" }}>
            SageDock couldn't open your browser. Visit github.com/yassineisa to see the source.
          </p>
        )}
      </section>

      <section className="card" aria-label="Open-source software">
        <span className="section-label">Attribution</span>
        <h2>Open-source software in SageDock</h2>
        <p style={{ marginTop: "var(--sp-1)" }}>
          SageDock is built on software written and maintained by other people, including the
          SageMath environment it installs and the Python, JupyterLab, and scientific packages that
          come with it. Each part keeps its own license and its own copyright holders, and nothing
          in SageDock's license changes that.
        </p>

        <dl className="credit-list">
          {CREDITS.map((credit) => (
            <div className="credit-row" key={credit.name}>
              <dt>{credit.name}</dt>
              <dd>{credit.license}</dd>
            </div>
          ))}
        </dl>

        <p className="small" style={{ marginTop: "var(--sp-3)" }}>
          This list names the main components. The full license text for every package is included
          inside the computing environment. See LICENSE and THIRD-PARTY-NOTICES.md alongside the
          application for the complete terms.
        </p>
      </section>

      <div className="banner banner--warning" role="note">
        <Icon name="info" size={16} />
        <div className="banner__body">
          <p className="banner__title">Your notebooks are yours</p>
          <p className="banner__message">
            Notebooks are saved as ordinary Windows files in your Documents folder, outside the
            computing environment, so they survive reinstalling or repairing SageDock.
          </p>
        </div>
      </div>
    </>
  );
}
