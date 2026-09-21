import { HashRouter, Route, Routes } from "react-router-dom";
import { AppShell } from "./components/AppShell";
import { ConfigProvider, useConfig } from "./state/ConfigContext";
import { Home } from "./pages/Home";
import { Onboarding } from "./pages/Onboarding";
import { Settings } from "./pages/Settings";
import { Diagnostics } from "./pages/Diagnostics";
import { Help } from "./pages/Help";
import { About } from "./pages/About";
import { Recovery } from "./pages/Recovery";
import { Tools } from "./pages/Tools";
import { TaskProvider } from "./state/TaskContext";
import { SetupProvider } from "./state/SetupContext";

/**
 * Either the first-run introduction or the app itself, never both.
 *
 * Separated from `App` so it sits inside `ConfigProvider` and can read the saved setting.
 * The introduction replaces the whole shell rather than appearing over it — see
 * `Onboarding` for why.
 */
function Root() {
  const { config } = useConfig();

  // Settings have not arrived yet. Rendering the app and then replacing it with the
  // introduction a moment later would flash the interface at exactly the person least
  // equipped to make sense of it.
  //
  // This blank is bounded, and that bound matters: `ConfigProvider` falls back to defaults
  // after `SETTINGS_GRACE_MS`, so a settings read that stalls or never settles cannot leave
  // SageDock as a blank window with no error and no way out.
  if (!config) return null;

  if (!config.onboarding_complete) return <Onboarding />;

  return (
    <HashRouter>
      <AppShell>
        <Routes>
          <Route path="/" element={<Home />} />
          <Route path="/settings" element={<Settings />} />
          <Route path="/diagnostics" element={<Diagnostics />} />
          <Route path="/help" element={<Help />} />
          <Route path="/about" element={<About />} />
          <Route path="/recovery" element={<Recovery />} />
          <Route path="/tools" element={<Tools />} />
        </Routes>
      </AppShell>
    </HashRouter>
  );
}

export default function App() {
  return (
    <ConfigProvider>
      <TaskProvider>
        {/* Above the router on purpose. Setup belongs to the backend and outlives every
            screen, so the thing observing it has to outlive every screen too — a provider
            mounted inside a route would lose its subscription the moment the user
            navigated, which is exactly the bug this replaces. */}
        <SetupProvider>
          <Root />
        </SetupProvider>
      </TaskProvider>
    </ConfigProvider>
  );
}
