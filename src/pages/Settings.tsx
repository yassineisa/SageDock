import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { useConfig } from "../state/ConfigContext";
import { useTask } from "../state/TaskContext";
import { commands, type Browser, type ThemePreference } from "../lib/commands";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { Icon } from "../components/Icon";

const THEMES: ThemePreference[] = ["system", "light", "dark"];

export function Settings() {
  const { config, setTheme, setBrowser, setPreferredBrowser, setOnboardingComplete } = useConfig();
  const task = useTask();
  const [quit, setQuit] = useState(false);
  const [browsers, setBrowsers] = useState<Browser[]>([]);

  // Only read the registry when the choice is actually on screen. Failing to list browsers
  // leaves the picker showing "Your default browser" alone, which is the behaviour SageDock
  // had before this setting existed, not an error worth interrupting Settings for.
  const usingBrowser = !!config?.open_in_browser;
  useEffect(() => {
    if (!usingBrowser) return;
    let cancelled = false;
    commands
      .installedBrowsers()
      .then((found) => {
        if (!cancelled) setBrowsers(found);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [usingBrowser]);

  return (
    <>
      <header className="page-head">
        <div>
          <h1>Settings</h1>
          <p>Appearance, notebooks, and your computing environment.</p>
        </div>
      </header>

      <section aria-label="Appearance">
        <span className="section-label">Appearance</span>
        <div className="settings-group">
          <div className="setting-row">
            <div className="setting-main">
              <Icon name="settings" size={20} />
              <div>
                <h3>Theme</h3>
                <p>Follow Windows, or choose light or dark.</p>
              </div>
            </div>
            <div className="segmented" role="group" aria-label="Theme">
              {THEMES.map((theme) => (
                <button
                  key={theme}
                  aria-pressed={config?.theme === theme}
                  disabled={!!task.busy}
                  onClick={() => void task.run("Saving theme", () => setTheme(theme))}
                >
                  {theme[0].toUpperCase() + theme.slice(1)}
                </button>
              ))}
            </div>
          </div>

          <div className="setting-row">
            <div className="setting-main">
              <Icon name="openFile" size={20} />
              <div>
                <h3>Open notebooks in</h3>
                <p>A dedicated SageDock window, or your default browser.</p>
              </div>
            </div>
            <div className="segmented" role="group" aria-label="Notebook destination">
              <button
                aria-pressed={!config?.open_in_browser}
                disabled={!!task.busy}
                onClick={() => void task.run("Saving preference", () => setBrowser(false))}
              >
                SageDock
              </button>
              <button
                aria-pressed={!!config?.open_in_browser}
                disabled={!!task.busy}
                onClick={() => void task.run("Saving preference", () => setBrowser(true))}
              >
                Browser
              </button>
            </div>
          </div>

          {/* Hidden rather than disabled when notebooks open in SageDock's own window: a
              browser picker that cannot affect anything is a control that misleads. */}
          {usingBrowser && (
            <div className="setting-row">
              <div className="setting-main">
                <Icon name="openFile" size={20} />
                <div>
                  <h3>Which browser</h3>
                  <p>SageDock found these installed on this PC.</p>
                </div>
              </div>
              <select
                className="input setting-select"
                aria-label="Which browser"
                disabled={!!task.busy}
                value={config?.preferred_browser ?? ""}
                onChange={(e) => {
                  const value = e.target.value || null;
                  void task.run("Saving preference", () => setPreferredBrowser(value));
                }}
              >
                <option value="">Your default browser</option>
                {browsers.map((browser) => (
                  <option key={browser.id} value={browser.id}>
                    {browser.name}
                  </option>
                ))}
              </select>
            </div>
          )}
        </div>
      </section>

      <section aria-label="Files and environment">
        <span className="section-label">Files and environment</span>
        <div className="settings-group">
          <div className="setting-row">
            <div className="setting-main">
              <Icon name="folder" size={20} />
              <div>
                <h3>Workspaces folder</h3>
                <p>
                  The folder holding your course workspaces. Ordinary Windows files, yours to keep,
                  move, and back up.
                </p>
              </div>
            </div>
            <button
              className="btn"
              disabled={!!task.busy}
              onClick={() => void task.run("Opening folder", commands.openWorkspacesFolder)}
            >
              Open folder
            </button>
          </div>

          <div className="setting-row">
            <div className="setting-main">
              <Icon name="repair" size={20} />
              <div>
                <h3>Environment maintenance</h3>
                <p>Check, reinstall, or restore the computing environment.</p>
              </div>
            </div>
            <Link className="btn" to="/recovery">
              Open Recovery
            </Link>
          </div>

          <div className="setting-row">
            <div className="setting-main">
              <Icon name="info" size={20} />
              <div>
                <h3>Updates</h3>
                <p>
                  Install a newer SageDock from your trusted distributor. Saved notebooks stay in
                  Windows.
                </p>
              </div>
            </div>
            <Link className="btn" to="/help">
              Update guide
            </Link>
          </div>
        </div>
      </section>

      <section aria-label="Session">
        <span className="section-label">Session</span>
        <div className="settings-group">
          {/* Without this, skipping the introduction once would put it permanently out of
              reach, which is the wrong trade for a screen that explains the product. */}
          <div className="setting-row">
            <div className="setting-main">
              <Icon name="help" size={20} />
              <div>
                <h3>Introduction</h3>
                <p>Show the first-run introduction again the next time SageDock opens.</p>
              </div>
            </div>
            <button
              className="btn"
              disabled={!!task.busy}
              onClick={() =>
                void task.run("Saving preference", async () => {
                  await setOnboardingComplete(false);
                  return "The introduction will appear the next time SageDock opens.";
                })
              }
            >
              Show again
            </button>
          </div>

          <div className="setting-row">
            <div className="setting-main">
              <Icon name="stop" size={20} />
              <div>
                <h3>Stop SageMath and close</h3>
                <p>Frees the memory SageMath is using and closes SageDock.</p>
              </div>
            </div>
            <button className="btn" disabled={!!task.busy} onClick={() => setQuit(true)}>
              Stop and close
            </button>
          </div>
        </div>
      </section>

      {quit && (
        <ConfirmDialog
          title="Close SageDock?"
          confirm="Stop and close"
          busy={!!task.busy}
          onCancel={() => setQuit(false)}
          onConfirm={() => void task.run("Closing SageDock", commands.shutdownAndQuit)}
        >
          <p>
            <strong>Save every open notebook first.</strong> Running calculations will stop and
            browser tabs will disconnect.
          </p>
          <p>Files already saved on your computer are kept.</p>
        </ConfirmDialog>
      )}
    </>
  );
}
