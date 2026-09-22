import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { commands, isAppError, type AppConfig, type ThemePreference } from "../lib/commands";

type ResolvedTheme = "light" | "dark";

/**
 * How long the app waits for settings before showing itself anyway.
 *
 * Reading settings is one IPC call that normally answers in well under a millisecond, so a
 * healthy launch never reaches this. It exists because the first-run introduction is
 * decided by the result: without a bound, a call that never settles would leave SageDock as
 * a blank window with no error and no way out of it. Showing the app with defaults is a far
 * better failure than showing nothing at all.
 */
const SETTINGS_GRACE_MS = 1500;

/**
 * What to use when settings cannot be read, or do not arrive in time.
 *
 * `onboarding_complete` is deliberately `true`. In this state settings are not writable, so
 * finishing the introduction could not be recorded and it would return on every launch, a
 * first-run screen nobody can get past is worse than not showing one.
 */
const FALLBACK_CONFIG: AppConfig = {
  schema_version: 1,
  theme: "system",
  open_in_browser: false,
  onboarding_complete: true,
  preferred_browser: null,
};

interface ConfigContextValue {
  config: AppConfig | null;
  /** Set when the initial config load itself failed (defaults are shown either way). */
  loadError: string | null;
  setTheme: (theme: ThemePreference) => Promise<void>;
  setBrowser: (value: boolean) => Promise<void>;
  /** `null` means "whatever Windows has set as the default browser". */
  setPreferredBrowser: (value: string | null) => Promise<void>;
  setOnboardingComplete: (value: boolean) => Promise<void>;
}

const ConfigContext = createContext<ConfigContextValue | null>(null);

function resolveTheme(
  theme: ThemePreference | undefined,
  systemPrefersDark: boolean,
): ResolvedTheme {
  if (theme === "light") return "light";
  if (theme === "dark") return "dark";
  return systemPrefersDark ? "dark" : "light";
}

export function ConfigProvider({ children }: { children: ReactNode }) {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [systemPrefersDark, setSystemPrefersDark] = useState(
    () => window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false,
  );

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const listener = (event: MediaQueryListEvent) => setSystemPrefersDark(event.matches);
    media.addEventListener("change", listener);
    return () => media.removeEventListener("change", listener);
  }, []);

  useEffect(() => {
    let cancelled = false;
    let usedFallback = false;

    const timer = setTimeout(() => {
      if (cancelled) return;
      usedFallback = true;
      setLoadError("Settings are taking longer than expected, so SageDock is using defaults.");
      setConfig(FALLBACK_CONFIG);
    }, SETTINGS_GRACE_MS);

    commands
      .getConfig()
      .then((loaded) => {
        if (cancelled) return;
        clearTimeout(timer);
        // If the app is already on screen under defaults, take the real settings but do
        // not now send the student into an introduction they have just been shown past.
        setConfig(usedFallback ? { ...loaded, onboarding_complete: true } : loaded);
      })
      .catch((err) => {
        if (cancelled) return;
        clearTimeout(timer);
        // The backend never fails to load config (it falls back to defaults itself),
        // so this only fires if the IPC call itself couldn't reach the backend.
        setLoadError(isAppError(err) ? err.message : "Settings could not be loaded.");
        setConfig(FALLBACK_CONFIG);
      });

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, []);

  const resolvedTheme = resolveTheme(config?.theme, systemPrefersDark);

  useEffect(() => {
    document.documentElement.dataset.theme = resolvedTheme;
  }, [resolvedTheme]);

  const setTheme = useCallback(async (theme: ThemePreference) => {
    const updated = await commands.setTheme(theme);
    setConfig(updated);
  }, []);

  const value = useMemo(
    () => ({
      config,
      loadError,
      setTheme,
      setBrowser: async (value: boolean) => {
        setConfig(await commands.setBrowserPreference(value));
      },
      setPreferredBrowser: async (value: string | null) => {
        setConfig(await commands.setPreferredBrowser(value));
      },
      setOnboardingComplete: async (value: boolean) => {
        setConfig(await commands.setOnboardingComplete(value));
      },
    }),
    [config, loadError, setTheme],
  );

  return <ConfigContext.Provider value={value}>{children}</ConfigContext.Provider>;
}

export function useConfig(): ConfigContextValue {
  const ctx = useContext(ConfigContext);
  if (!ctx) throw new Error("useConfig must be used within a ConfigProvider");
  return ctx;
}
