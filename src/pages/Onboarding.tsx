import { useEffect, useState } from "react";
import { commands, friendlyError, type Browser, type ThemePreference } from "../lib/commands";
import { useConfig } from "../state/ConfigContext";
import { ErrorBanner } from "../components/ErrorBanner";
import { Icon, type IconName } from "../components/Icon";

/**
 * The first-run introduction.
 *
 * Shown instead of the app, not as a dialog over it, because a student meeting SageDock
 * for the first time has nothing to look at behind a dialog, and a modal invites dismissal
 * before anything has been read. It renders outside `AppShell` for the same reason: a
 * navigation rail full of pages that mean nothing yet is noise on this screen.
 *
 * The explanatory steps deliberately come before the choices. The last step is the only one
 * that changes anything, and every setting it collects is also in Settings, so nothing here
 * is a decision the student is locked into.
 */

const THEMES: { value: ThemePreference; label: string }[] = [
  { value: "system", label: "Match Windows" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

/** The three explanatory steps. The fourth step is the setup form, which is not a card. */
const STEPS: { icon: IconName; title: string; body: string }[] = [
  {
    icon: "calculator",
    title: "SageMath and Python, without a terminal",
    body: "SageDock installs and manages its own Linux computing environment containing SageMath, Python, and JupyterLab. You never have to open a terminal, install packages, or configure anything to use them. Setting it up is a single button on the Home screen.",
  },
  {
    icon: "folder",
    title: "Your work stays in ordinary Windows folders",
    body: "Notebooks are saved in plain folders inside your Documents, outside the computing environment, so they stay visible in File Explorer and get picked up by whatever backup you already use. Repairing or reinstalling the environment never touches them. You can keep a separate workspace folder per course.",
  },
  {
    icon: "openFile",
    title: "Notebooks, downloads, and backups in one place",
    body: "The Home screen lists the notebooks you opened recently and anything in your Downloads folder, so a notebook a lecturer sent you is two clicks from running. Create backup writes every workspace to a single file you can restore on another computer.",
  },
];

export function Onboarding() {
  const { config, setTheme, setBrowser, setPreferredBrowser, setOnboardingComplete } = useConfig();
  const [step, setStep] = useState(0);
  const [browsers, setBrowsers] = useState<Browser[]>([]);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<ReturnType<typeof friendlyError> | null>(null);

  const last = STEPS.length;
  const onSetupStep = step === last;

  // Read once the setup step is in view rather than on mount: a registry scan the student
  // may never reach is work that doesn't need doing, and this keeps the first paint clean.
  useEffect(() => {
    if (!onSetupStep) return;
    let cancelled = false;
    commands
      .installedBrowsers()
      .then((found) => {
        if (!cancelled) setBrowsers(found);
      })
      // Not being able to list browsers is not worth blocking setup over: the choice simply
      // falls back to "whatever Windows has set", which is what SageDock always did before.
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [onSetupStep]);

  // Failing to record this is not fatal, but it does mean the introduction returns on the
  // next launch, so it is reported rather than swallowed.
  const finish = () => {
    setSaving(true);
    setError(null);
    void setOnboardingComplete(true)
      .catch((err) => setError(friendlyError(err)))
      .finally(() => setSaving(false));
  };

  const change = (action: () => Promise<void>) => {
    setError(null);
    void action().catch((err) => setError(friendlyError(err)));
  };

  return (
    <div className="onboarding">
      <main className="onboarding-panel" aria-labelledby="onboarding-title">
        <div className="onboarding-brand">
          <span className="brand-name">SageDock</span>
          <span className="onboarding-progress" role="status">
            Step {step + 1} of {last + 1}
          </span>
        </div>

        {error && (
          <ErrorBanner
            title={error.title}
            message={error.message}
            technicalDetails={error.details}
          />
        )}

        {!onSetupStep ? (
          <div className="onboarding-step">
            <Icon name={STEPS[step].icon} size={28} />
            <h1 id="onboarding-title">{STEPS[step].title}</h1>
            <p>{STEPS[step].body}</p>
          </div>
        ) : (
          <div className="onboarding-step">
            <Icon name="settings" size={28} />
            <h1 id="onboarding-title">Set up how SageDock looks and opens your work</h1>
            <p>Both of these are in Settings afterwards, so nothing here is permanent.</p>

            <div className="onboarding-fields">
              <div className="onboarding-field">
                <label htmlFor="onboarding-theme">
                  <strong>Appearance</strong>
                  <span>Follow Windows, or pick light or dark.</span>
                </label>
                <div className="segmented" role="group" aria-label="Appearance">
                  {THEMES.map((theme) => (
                    <button
                      key={theme.value}
                      id={theme.value === "system" ? "onboarding-theme" : undefined}
                      aria-pressed={config?.theme === theme.value}
                      disabled={saving}
                      onClick={() => change(() => setTheme(theme.value))}
                    >
                      {theme.label}
                    </button>
                  ))}
                </div>
              </div>

              <div className="onboarding-field">
                <label htmlFor="onboarding-destination">
                  <strong>Where notebooks open</strong>
                  <span>A dedicated SageDock window, or a web browser.</span>
                </label>
                <div className="segmented" role="group" aria-label="Where notebooks open">
                  <button
                    id="onboarding-destination"
                    aria-pressed={!config?.open_in_browser}
                    disabled={saving}
                    onClick={() => change(() => setBrowser(false))}
                  >
                    SageDock
                  </button>
                  <button
                    aria-pressed={!!config?.open_in_browser}
                    disabled={saving}
                    onClick={() => change(() => setBrowser(true))}
                  >
                    Browser
                  </button>
                </div>
              </div>

              {/* Only meaningful once "Browser" is chosen, and hidden otherwise rather than
                  disabled: a browser picker beside a notebook that opens in SageDock's own
                  window is a control with no effect. */}
              {config?.open_in_browser && (
                <div className="onboarding-field">
                  <label htmlFor="onboarding-browser">
                    <strong>Which browser</strong>
                    <span>SageDock found these on this PC.</span>
                  </label>
                  <select
                    id="onboarding-browser"
                    className="input"
                    disabled={saving}
                    value={config?.preferred_browser ?? ""}
                    onChange={(e) => change(() => setPreferredBrowser(e.target.value || null))}
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
          </div>
        )}

        <div className="onboarding-actions">
          {/* Skipping is a first-class action, not hidden text: someone reinstalling on a
              second machine should not have to page through an introduction they know. */}
          <button className="btn btn-subtle" disabled={saving} onClick={finish}>
            Skip introduction
          </button>
          <span className="spacer" />
          {step > 0 && (
            <button className="btn" disabled={saving} onClick={() => setStep(step - 1)}>
              Back
            </button>
          )}
          {onSetupStep ? (
            <button className="btn btn-accent" disabled={saving} onClick={finish}>
              <Icon name="accept" size={16} />
              {saving ? "Working…" : "Start using SageDock"}
            </button>
          ) : (
            <button className="btn btn-accent" disabled={saving} onClick={() => setStep(step + 1)}>
              Next
              <Icon name="chevronRight" size={16} />
            </button>
          )}
        </div>
      </main>
    </div>
  );
}
