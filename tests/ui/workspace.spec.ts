import { test, expect, type Page } from "@playwright/test";
import packageInfo from "../../package.json" with { type: "json" };

const appVersion = packageInfo.version;

// These are UI contract tests against a mocked backend. They prove the screens render,
// wire up, and stay keyboard-reachable. They prove NOTHING about whether Windows setup,
// WSL, SageMath, real backups, or real workspace folders work — only the Rust tests and a
// real install can do that.

type Options = {
  installed?: boolean;
  running?: boolean;
  configUnavailable?: boolean;
  jupyterFails?: boolean;
  /**
   * Which tool picture the backend reports. `partial` is the interesting one: some
   * components of a group present and working, others missing.
   */
  tools?: "verified" | "partial" | "none" | "unavailable";
  /** The first install attempt fails its verification, the next succeeds. */
  installFailsOnce?: boolean;
  /**
   * What the Downloads list reports. Empty exercises the empty state.
   *
   * `folder` and `notebook` are what the backend adds for a file the built-in browser
   * downloaded somewhere other than the Downloads folder: the containing folder's name,
   * and whether it is a notebook at all.
   */
  downloads?: {
    name: string;
    path: string;
    modified: number;
    folder?: string | null;
    notebook?: boolean;
  }[];
  /** Renaming is refused because a notebook is open in that very workspace. */
  renameBlocked?: boolean;
  /**
   * File names already present in a workspace, keyed by workspace id. Drives the
   * name-collision dialog for a downloaded notebook: a workspace not listed here has no
   * conflicts.
   */
  existingWorkspaceFiles?: Record<string, string[]>;
  /** Notebooks reported as recently opened in the active workspace. */
  recent?: { name: string; path: string; modified: number }[];
  /**
   * What the native "Open notebook" picker reports as chosen. A real file dialog can't be
   * driven from a test, so this stands in for the student's choice; `null` simulates
   * closing the picker without choosing anything.
   */
  pickedFile?: string | null;
  /**
   * Start at the first-run introduction instead of the app. Every other test runs against
   * an installation that has already been through it, which is what a second launch is.
   */
  showOnboarding?: boolean;
  /** What the registry scan reports. Empty exercises the "default browser only" fallback. */
  browsers?: { id: string; name: string }[];
};

async function fixture(
  page: Page,
  {
    installed = true,
    running = true,
    configUnavailable = false,
    jupyterFails = false,
    tools = "verified",
    installFailsOnce = false,
    downloads = [
      { name: "Assignment 1", path: "Assignment 1.ipynb", modified: 1789473600 },
      // What a browser actually writes: the backend reports the real filename, and strips
      // the extension chain for display.
      { name: "Week 02 Lab", path: "Week 02 Lab.ipynb.json", modified: 1789387200 },
    ],
    renameBlocked = false,
    existingWorkspaceFiles = {},
    recent = [
      {
        name: "Exploring prime numbers",
        path: "Notebooks/Exploring prime numbers.ipynb",
        modified: 1789473600,
      },
      {
        name: "A first look at our solar system",
        path: "Notebooks/A first look at our solar system.ipynb",
        modified: 1789387200,
      },
      {
        name: "Calculus · Week 01",
        path: "Notebooks/Calculus · Week 01.ipynb",
        modified: 1789300800,
      },
    ],
    pickedFile = "Physics Homework.ipynb",
    showOnboarding = false,
    browsers = [
      { id: "Google Chrome", name: "Google Chrome" },
      { id: "Firefox-308046B0AF4A39CB", name: "Mozilla Firefox" },
    ],
  }: Options = {},
) {
  await page.addInitScript(
    ({
      installed,
      running,
      appVersion,
      configUnavailable,
      jupyterFails,
      tools,
      installFailsOnce,
      downloads,
      renameBlocked,
      existingWorkspaceFiles,
      recent,
      pickedFile,
      showOnboarding,
      browsers,
    }) => {
      let pickedFileName: string | null = pickedFile;
      let theme = "light";
      let browser = false;
      // The introduction is gating, so the mock has to model it the way the backend does:
      // one saved flag, written by the same command Settings uses to show it again.
      let onboardingComplete = !showOnboarding;
      let preferredBrowser: string | null = null;
      const configValue = () => ({
        schema_version: 1,
        theme,
        open_in_browser: browser,
        onboarding_complete: onboardingComplete,
        preferred_browser: preferredBrowser,
      });
      let ready = installed;
      let envRunning = running;

      // Tools are modelled the way the backend reports them: per-component results, with
      // each group's state derived from them exactly as `scientific::state_of` does. That
      // keeps the mock honest about the one distinction that matters — a partly installed
      // group is neither "installed" nor "not installed".
      const COMPONENT_LABEL: Record<string, string> = {
        gcc: "C compiler (gcc)",
        gxx: "C++ compiler (g++)",
        gfortran: "Fortran compiler (gfortran)",
        make: "Make",
        cmake: "CMake",
        pkg_config: "pkg-config",
      };
      const GROUPS: Record<string, string[]> = {
        cpp: ["gcc", "gxx"],
        fortran: ["gfortran"],
        build: ["make", "cmake", "pkg_config"],
        toolkit: ["gcc", "gxx", "gfortran", "make", "cmake", "pkg_config"],
      };
      const BY_MODE: Record<string, Record<string, boolean>> = {
        verified: {
          gcc: true,
          gxx: true,
          gfortran: true,
          make: true,
          cmake: true,
          pkg_config: true,
        },
        // C and C++ work and Make works; CMake and pkg-config are missing.
        partial: { gcc: true, gxx: true, make: true },
        none: {},
        // The machine has everything; SageDock simply has not been able to look yet. That
        // gap between "unknown" and "absent" is the whole point of this mode, so a forced
        // check has to be able to find them.
        unavailable: {
          gcc: true,
          gxx: true,
          gfortran: true,
          make: true,
          cmake: true,
          pkg_config: true,
        },
      };
      const working: Record<string, boolean> = { ...(BY_MODE[tools] ?? {}) };
      let toolSource = tools === "unavailable" ? "unavailable" : "verified";
      let installAttempts = 0;

      const buildReport = () => {
        const group = (id: string) => {
          const components = GROUPS[id].map((cid) => ({
            id: cid,
            label: COMPONENT_LABEL[cid],
            present: !!working[cid],
            works: !!working[cid],
            version: working[cid] ? "13.2.0" : null,
            detail: working[cid] ? null : "not found",
          }));
          const state = components.every((c) => c.works)
            ? "installed"
            : components.some((c) => c.works || c.present)
              ? "needs_repair"
              : "not_installed";
          return {
            id,
            state,
            version: components.find((c) => c.works)?.version ?? null,
            components,
          };
        };
        return {
          source: toolSource,
          checked_at: toolSource === "unavailable" ? null : "2026-09-18T00:00:00Z",
          reason:
            toolSource === "verified"
              ? null
              : "SageMath is stopped, so SageDock didn't start it just to check.",
          tools:
            toolSource === "unavailable"
              ? []
              : [
                  group("cpp"),
                  group("fortran"),
                  group("build"),
                  group("toolkit"),
                  { id: "seaborn", state: "installed", version: "0.13.2", components: [] },
                  { id: "statsmodels", state: "not_installed", version: null, components: [] },
                  { id: "polars", state: "not_installed", version: null, components: [] },
                ],
        };
      };
      const calls: string[] = [];
      const requests: { command: string; args: Record<string, any> }[] = [];
      const callbacks: Record<number, Function> = {};
      let id = 0;

      // Mirrors `library::target_notebook_name`: a browser-saved `.json` notebook lands in
      // a workspace under the same name it would if it had been `.ipynb` all along.
      const downloadTargetName = (fileName: string) => {
        const withoutJson = fileName.endsWith(".json") ? fileName.slice(0, -5) : fileName;
        const stem = withoutJson.endsWith(".ipynb") ? withoutJson.slice(0, -6) : withoutJson;
        return `${stem}.ipynb`;
      };
      // Mutated as notebooks are opened, mirroring what the real workspace folders would
      // contain — this is what makes the name-collision dialog appear only when it should.
      const existingFiles: Record<string, string[]> = existingWorkspaceFiles;

      let workspaces = [
        {
          id: "ws-1",
          name: "Calculus",
          path: "C:\\Users\\Student\\Documents\\SageDock Workspaces\\Calculus",
          last_opened: 1789473600,
          is_active: true,
          status: "available",
        },
        {
          id: "ws-2",
          name: "Physics",
          path: "C:\\Users\\Student\\Documents\\SageDock Workspaces\\Physics",
          last_opened: null,
          is_active: false,
          status: "available",
        },
        {
          id: "ws-3",
          name: "Old Laptop Folder",
          path: "D:\\Gone\\Statistics",
          last_opened: 1789300800,
          is_active: false,
          status: "missing",
        },
      ];

      Object.defineProperty(window, "__TAURI_INTERNALS__", {
        value: {
          // `getCurrentWebview()` reads these. Without them it throws synchronously, and
          // the drag-and-drop subscription would be skipped rather than exercised.
          metadata: {
            currentWindow: { label: "main" },
            currentWebview: { windowLabel: "main", label: "main" },
          },
          transformCallback: (fn: Function) => {
            callbacks[++id] = fn;
            return id;
          },
          unregisterCallback: () => {},
          invoke: async (command: string, args: Record<string, any> = {}) => {
            calls.push(command);
            requests.push({ command, args });
            if (command.startsWith("plugin:event|")) return 1;
            if (command === "get_config") {
              if (configUnavailable) throw new Error("Settings backend unavailable");
              return configValue();
            }
            if (command === "set_theme") {
              theme = String(args.theme);
              return configValue();
            }
            if (command === "set_browser_preference") {
              browser = Boolean(args.value);
              return configValue();
            }
            if (command === "installed_browsers") return browsers;
            if (command === "set_preferred_browser") {
              preferredBrowser = args.value === null ? null : String(args.value);
              return configValue();
            }
            if (command === "set_onboarding_complete") {
              onboardingComplete = Boolean(args.value);
              return configValue();
            }
            if (command === "get_app_info")
              return {
                name: "SageDock",
                version: appVersion,
                developer: "Yassin Eisa",
                license: "MIT",
              };
            if (command === "get_setup_status")
              return {
                environment_ready: ready,
                environment_installed: ready,
                awaiting_restart: false,
                workspace_path: "C:\\Users\\Student\\Documents\\SageDock",
                sage_package: {
                  file_name: "SageMath package",
                  sage_version: "10.9",
                  has_checksum: true,
                  folder: "",
                },
                problem: null,
                busy: false,
              };
            if (command === "environment_status")
              return {
                installed: ready,
                running: ready && envRunning,
                notebooks_running: ready && envRunning,
                open_workspaces: envRunning ? 1 : 0,
                busy: null,
              };
            if (command === "stop_environment") {
              envRunning = false;
              return {
                installed: ready,
                running: false,
                notebooks_running: false,
                open_workspaces: 0,
                busy: null,
              };
            }
            if (command === "list_workspaces") return workspaces;
            if (command === "create_workspace") {
              const created = {
                id: "ws-new",
                name: String(args.name),
                path: "C:\\Users\\Student\\Documents\\SageDock Workspaces\\" + args.name,
                last_opened: null,
                is_active: false,
                status: "available",
              };
              workspaces = [...workspaces, created];
              return created;
            }
            if (command === "rename_workspace") {
              // Renaming moves the folder, so a notebook open inside that folder refuses it.
              if (renameBlocked)
                throw {
                  code: "WORKSPACE_IN_USE",
                  severity: "error",
                  component: "workspace",
                  user_files_safe: true,
                  title: "Stop SageMath before renaming a workspace",
                  message:
                    "Renaming moves the folder on disk, and a notebook is still open inside it. Use Stop SageMath on the Home screen, then rename it. Nothing has been changed.",
                };
              workspaces = workspaces.map((w) =>
                w.id === args.id ? { ...w, name: String(args.name) } : w,
              );
              return workspaces.find((w) => w.id === args.id);
            }
            if (command === "add_files_to_workspace") return 2;
            if (command === "downloaded_notebooks") return downloads;
            if (command === "check_download_target") {
              const targetName = downloadTargetName(String(args.name));
              const ws = workspaces.find((w) => w.id === args.workspaceId);
              const files = existingFiles[args.workspaceId] ?? [];
              return {
                workspace_name: ws ? ws.name : "",
                target_name: targetName,
                exists: files.includes(targetName),
              };
            }
            if (command === "open_downloaded_notebook") {
              const targetName = downloadTargetName(String(args.name));
              const files = (existingFiles[args.workspaceId] ??= []);
              let landed: string;
              if (args.onConflict === "open_existing" && files.includes(targetName)) {
                landed = targetName;
              } else if (args.onConflict === "replace") {
                landed = targetName;
                if (!files.includes(targetName)) files.push(targetName);
              } else if (!files.includes(targetName)) {
                landed = targetName;
                files.push(targetName);
              } else {
                // The real backend's dedup loop: keep counting until a free name is found.
                let n = 1;
                let candidate = targetName.replace(/\.ipynb$/, ` (${n}).ipynb`);
                while (files.includes(candidate)) {
                  n += 1;
                  candidate = targetName.replace(/\.ipynb$/, ` (${n}).ipynb`);
                }
                landed = candidate;
                files.push(candidate);
              }
              envRunning = true;
              // Opening a workspace from Downloads makes it active, the same as Launch
              // workspace on its card.
              workspaces = workspaces.map((w) => ({ ...w, is_active: w.id === args.workspaceId }));
              return landed;
            }
            if (command === "start_notebook_drag") return;
            if (command === "forget_workspace") {
              workspaces = workspaces.filter((w) => w.id !== args.id);
              return workspaces;
            }
            if (command === "add_workspace_folder") return null;
            if (command === "launch_workspace" || command === "reveal_workspace") return;
            if (command === "recent_notebooks") return ready ? recent : [];
            if (command === "choose_notebook_file") return pickedFileName;
            if (command === "check_picked_target") {
              const targetName = downloadTargetName(pickedFileName ?? "");
              const ws = workspaces.find((w) => w.id === args.workspaceId);
              const files = existingFiles[args.workspaceId] ?? [];
              return {
                workspace_name: ws ? ws.name : "",
                target_name: targetName,
                exists: files.includes(targetName),
              };
            }
            if (command === "open_picked_notebook") {
              const name = pickedFileName ?? "Imported Notebook.ipynb";
              pickedFileName = null;
              const targetName = downloadTargetName(name);
              const files = (existingFiles[args.workspaceId] ??= []);
              let landed: string;
              if (args.onConflict === "open_existing" && files.includes(targetName)) {
                landed = targetName;
              } else if (args.onConflict === "replace") {
                landed = targetName;
                if (!files.includes(targetName)) files.push(targetName);
              } else if (!files.includes(targetName)) {
                landed = targetName;
                files.push(targetName);
              } else {
                let n = 1;
                let candidate = targetName.replace(/\.ipynb$/, ` (${n}).ipynb`);
                while (files.includes(candidate)) {
                  n += 1;
                  candidate = targetName.replace(/\.ipynb$/, ` (${n}).ipynb`);
                }
                landed = candidate;
                files.push(candidate);
              }
              envRunning = true;
              workspaces = workspaces.map((w) => ({ ...w, is_active: w.id === args.workspaceId }));
              return landed;
            }
            if (command === "delete_recent_notebook") {
              recent = recent.filter((r) => r.path !== args.path);
              return;
            }
            if (command === "reveal_recent_notebook") return;
            if (command === "delete_downloaded_file") {
              downloads = downloads.filter((d) => d.path !== args.name);
              return;
            }
            if (command === "open_downloaded_file_externally") return;
            if (command === "reveal_downloaded_file") return;
            if (command === "run_setup") {
              ready = true;
              return "ready";
            }
            if (command === "new_notebook") return { relative_path: "Test.ipynb" };
            if (command === "open_notebook") {
              if (jupyterFails)
                throw {
                  code: "JUPYTER_READY_TIMEOUT",
                  severity: "error",
                  title: "The notebook service is taking too long to start",
                  message: "Your saved notebooks are safe. Try again.",
                };
              envRunning = true;
              return;
            }
            if (command === "create_backup")
              return {
                path: "D:\\backup.zip",
                file_count: 12,
                total_bytes: 3_145_728,
                workspaces: ["Calculus"],
                skipped: [],
              };
            if (command === "preview_backup")
              return {
                file_name: "SageDock-backup-2026-09-17.zip",
                preview: {
                  format: 1,
                  app_version: "1.1.0",
                  created_utc: "2026-09-17T09:00:00Z",
                  file_count: 12,
                  total_bytes: 3_145_728,
                  compatible: true,
                  note: null,
                  workspaces: [
                    {
                      name: "Calculus",
                      file_count: 12,
                      total_bytes: 3_145_728,
                      conflicts: true,
                      restored_as: "Calculus (restored)",
                    },
                  ],
                },
              };
            if (command === "restore_backup")
              return { restored: ["Calculus (restored)"], file_count: 12 };
            if (
              command === "open_workspaces_folder" ||
              command === "open_project_page" ||
              command === "shutdown_and_quit"
            )
              return;
            if (command === "scientific_tools") {
              // `force` is the user pressing "Check now"; only then may a stopped
              // environment be started, which is what turns an unknown status into a real one.
              if (args.force) toolSource = "verified";
              return buildReport();
            }
            if (command === "install_scientific_tool") {
              installAttempts += 1;
              if (installFailsOnce && installAttempts === 1)
                throw {
                  code: "TOOL_VERIFICATION_FAILED",
                  severity: "error",
                  title: "That tool installed but didn't pass its check",
                  message:
                    "SageDock installed the software, then tried to compile and run a small test program with it, and that didn't work.",
                };
              for (const id of GROUPS[String(args.tool)] ?? []) working[id] = true;
              toolSource = "verified";
              return buildReport();
            }
            if (command === "open_jupyter_home") {
              if (jupyterFails)
                throw {
                  code: "JUPYTER_READY_TIMEOUT",
                  severity: "error",
                  title: "The notebook service is taking too long to start",
                  message: "Your saved notebooks are safe. Try again.",
                };
              envRunning = true;
              return;
            }
            if (command === "recover") return "The computing environment passed its checks.";
            if (command === "diagnostic_report")
              return JSON.stringify({
                version: "1.1.1",
                checked_at: "2026-09-17",
                runtime: "healthy",
                checks: [
                  { check: "windows_version", severity: "info", summary: "Windows is ready." },
                ],
              });
            throw new Error("Unexpected command: " + command);
          },
        },
      });
      Object.defineProperty(window, "qaCalls", { get: () => calls });
      Object.defineProperty(window, "qaRequests", { get: () => requests });
      // Simulates a browser finishing a download while Home is already open — the poll
      // effect reads this on its next tick the same way it would read a real new file.
      Object.defineProperty(window, "qaAddDownload", {
        value: (entry: { name: string; path: string; modified: number }) => {
          downloads = [...downloads, entry];
        },
      });
    },
    {
      installed,
      running,
      appVersion,
      configUnavailable,
      jupyterFails,
      tools,
      installFailsOnce,
      downloads,
      renameBlocked,
      existingWorkspaceFiles,
      recent,
      pickedFile,
      showOnboarding,
      browsers,
    },
  );
}

test("workspace, notebook creation, search, and navigation", async ({ page }) => {
  await fixture(page);
  await page.goto("/");
  await expect(page.getByRole("button", { name: "New Sage Notebook" })).toBeVisible();
  await page.screenshot({ path: "docs/qa/workspace-light.png", fullPage: true });
  await page.getByLabel("Find a notebook").fill("prime");
  await expect(page.getByRole("button", { name: /Exploring prime numbers/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /solar system/ })).toHaveCount(0);
  await page.getByRole("button", { name: "New Sage Notebook" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).qaCalls.filter((x: string) => x === "open_notebook").length,
      ),
    )
    .toBe(1);
  await page.getByRole("link", { name: "Scientific tools", exact: false }).click();
  await expect(page.getByRole("heading", { name: "C / C++ compiler" })).toBeVisible();
});

test("new computer gets a clear setup path", async ({ page }) => {
  await fixture(page, { installed: false });
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Set up SageDock", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "New Sage Notebook" })).toHaveCount(0);
  await page.screenshot({ path: "docs/qa/setup-light.png", fullPage: true });
  await page.getByRole("button", { name: "Set up SageDock", exact: true }).click();
  await expect(page.getByRole("button", { name: "New Sage Notebook" })).toBeVisible();
});

test("the environment can be stopped from Home, with a warning to save first", async ({ page }) => {
  await fixture(page);
  await page.goto("/");

  await expect(page.getByText("Computing environment running")).toBeVisible();
  const stop = page.getByRole("button", { name: "Stop SageMath", exact: true });
  await expect(stop).toBeEnabled();
  await stop.click();

  // Stopping is destructive to unsaved work, so it must confirm and say so.
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText(/Save every open notebook first/)).toBeVisible();
  await expect(dialog.getByText(/frees the memory/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Cancel", exact: true })).toBeFocused();

  await dialog.getByRole("button", { name: "Stop SageMath", exact: true }).click();

  // The screen reflects the state the backend reported, not the fact a button was pressed.
  await expect(page.getByText("Computing environment stopped")).toBeVisible();
  await expect(page.getByRole("button", { name: "Stop SageMath", exact: true })).toBeDisabled();
});

test("backup restore and folder management work before SageMath setup", async ({ page }) => {
  await fixture(page, { installed: false, running: false });
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Create backup", exact: true })).toBeEnabled();
  await expect(page.getByRole("button", { name: "New workspace", exact: true })).toBeEnabled();
  await expect(page.getByRole("button", { name: "Launch workspace" }).first()).toBeDisabled();
  await page.getByRole("button", { name: "Restore backup", exact: true }).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Restore backup", exact: true })
    .click();
  await expect(page.getByText(/Restored 1 workspace/)).toBeVisible();
  expect(await page.evaluate(() => (window as any).qaCalls.includes("run_setup"))).toBe(false);
});

test("workspaces can be created, renamed, and removed without deleting files", async ({ page }) => {
  await fixture(page);
  await page.goto("/");

  await expect(page.getByRole("heading", { name: "Calculus" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Launch workspace" }).first()).toBeVisible();
  await expect(
    page
      .getByText("Opened today")
      .or(page.getByText(/Opened /))
      .first(),
  ).toBeVisible();

  // A folder that has moved says so in words, and can't be launched.
  const missingCard = page.locator(".workspace-card", { hasText: "Old Laptop Folder" });
  await expect(missingCard.getByText(/isn't where SageDock expects it/)).toBeVisible();
  await expect(missingCard.getByRole("button", { name: "Launch workspace" })).toBeDisabled();

  await page.getByRole("button", { name: "New workspace", exact: true }).click();
  await page.getByLabel("Workspace name").fill("Statistics");
  await page.getByRole("button", { name: "Create workspace", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Statistics" })).toBeVisible();

  const physics = page.locator(".workspace-card", { hasText: "Physics" });
  await physics.getByRole("button", { name: "Rename Physics" }).click();
  await page.getByLabel("New name for this workspace").fill("Physics 101");
  await page.getByRole("button", { name: "Save name", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Physics 101" })).toBeVisible();

  await page
    .locator(".workspace-card", { hasText: "Physics 101" })
    .getByRole("button", { name: /Remove Physics 101/ })
    .click();
  const confirm = page.getByRole("dialog");
  await expect(confirm.getByText(/Nothing will be deleted/)).toBeVisible();
  await confirm.getByRole("button", { name: "Remove from list", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Physics 101" })).toHaveCount(0);
});

test("restoring a backup previews its contents and never overwrites existing work", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/");

  await page.getByRole("button", { name: "Restore backup", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("SageDock-backup-2026-09-17.zip")).toBeVisible();
  await expect(dialog.getByText(/existing workspaces are never overwritten/)).toBeVisible();
  // The name it will actually land under is shown before anything is changed.
  await expect(dialog.getByText(/Calculus \(restored\)/)).toBeVisible();
  await expect(dialog.getByText(/you already have one with this name/)).toBeVisible();
  await dialog.getByRole("button", { name: "Restore backup", exact: true }).click();
  await expect(page.getByText(/Restored 1 workspace/)).toBeVisible();
});

test("theme, guides, about, and recovery confirmation work by keyboard", async ({ page }) => {
  await fixture(page);
  await page.goto("/#/settings");
  await page.getByRole("button", { name: "Dark", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.getByRole("link", { name: "Workspace", exact: false }).first().click();
  await expect(page.getByRole("button", { name: "New Sage Notebook" })).toBeVisible();
  await page.screenshot({ path: "docs/qa/workspace-dark.png", fullPage: true });

  await page.getByRole("link", { name: "Guides & help", exact: false }).click();
  await page.getByLabel("Search guides").fill("virtualization");
  await page.locator("summary").filter({ hasText: "When virtualization is turned off" }).click();
  await expect(page.getByText(/PC manufacturer's instructions/)).toBeVisible();

  // Attribution now lives on its own About page, reachable from the navigation pane.
  await page.getByRole("link", { name: "About", exact: true }).click();
  await expect(page.getByRole("heading", { name: "About", exact: true })).toBeVisible();
  await expect(page.getByRole("heading", { name: "SageDock", exact: true })).toBeVisible();
  await expect(page.getByText("Yassin Eisa").first()).toBeVisible();
  await expect(page.getByText(/MIT License/)).toBeVisible();
  await expect(page.getByText(/keeps its own license/)).toBeVisible();

  // Open-source attribution and the developer's source link both belong in the app itself,
  // not only in the repository files a student never sees.
  await expect(page.locator(".credit-row", { hasText: "SageMath" }).first()).toBeVisible();
  await expect(page.getByText("GPL-2.0-or-later", { exact: true })).toBeVisible();
  const projectLink = page.getByRole("button", { name: "github.com/yassineisa", exact: true });
  await expect(projectLink).toBeVisible();
  await page.screenshot({ path: "docs/qa/about-dark.png", fullPage: true });

  // The link must reach the backend, not merely look like a link.
  await projectLink.click();
  expect(await page.evaluate(() => (window as any).qaCalls.includes("open_project_page"))).toBe(
    true,
  );

  await page.getByRole("link", { name: "Recovery", exact: false }).first().click();
  await page.getByRole("button", { name: "Back up & reinstall", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByRole("button", { name: "Cancel", exact: true })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("diagnostics and compact window have no horizontal overflow", async ({ page }) => {
  await fixture(page);
  await page.goto("/#/diagnostics");
  await page.getByRole("button", { name: "Run diagnostics" }).click();
  await expect(page.getByText("Windows is ready.", { exact: true })).toBeVisible();
  await page.setViewportSize({ width: 860, height: 600 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
});

test("the Home screen has no horizontal overflow at a narrow width", async ({ page }) => {
  await fixture(page);
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Calculus" })).toBeVisible();
  await page.setViewportSize({ width: 860, height: 700 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
});

// Theme is changed by clicking through the app rather than navigating with goto: a full
// page load re-runs addInitScript, which resets the mocked preference back to light.
test("captures Settings and Home in both themes, including the minimum window size", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/settings");

  await page.getByRole("button", { name: "Light", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.screenshot({ path: "docs/qa/settings-light.png", fullPage: true });

  await page.getByRole("button", { name: "Dark", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.screenshot({ path: "docs/qa/settings-dark.png", fullPage: true });

  // tauri.conf.json declares minWidth 860 / minHeight 560, so the layout must hold there.
  await page.getByRole("link", { name: "Workspace", exact: false }).first().click();
  await page.setViewportSize({ width: 860, height: 560 });
  await expect(page.getByRole("button", { name: "New Sage Notebook" })).toBeVisible();
  await page.screenshot({ path: "docs/qa/home-minimum-dark.png", fullPage: true });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
});

test("captures the setup state in dark theme", async ({ page }) => {
  await fixture(page, { installed: false });
  await page.goto("/#/settings");
  await page.getByRole("button", { name: "Dark", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

  await page.getByRole("link", { name: "Workspace", exact: false }).first().click();
  await expect(page.getByRole("button", { name: "Set up SageDock", exact: true })).toBeVisible();
  await page.screenshot({ path: "docs/qa/setup-dark.png", fullPage: true });
});

test("attribution appears only on About, not on every screen", async ({ page }) => {
  await fixture(page);
  await page.setViewportSize({ width: 860, height: 560 });

  // There is no global footer any more, on any route.
  for (const route of ["/", "/#/settings", "/#/recovery", "/#/help"]) {
    await page.goto(route);
    await expect(page.getByRole("contentinfo")).toHaveCount(0);
    await expect(page.getByText("Created by Yassin Eisa")).toHaveCount(0);
  }

  // The version and the creator credit are still in the app, on About.
  await page.goto("/#/about");
  await expect(page.getByText("Created by Yassin Eisa " + appVersion)).toHaveCount(0);
  await expect(page.getByText(/Created by Yassin Eisa/)).toBeVisible();
  await expect(page.getByText(appVersion, { exact: true })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test("a settings IPC failure is visible while the app continues with defaults", async ({
  page,
}) => {
  await fixture(page, { configUnavailable: true });
  await page.goto("/");
  await expect(page.getByText("Settings are using defaults", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "New Sage Notebook" })).toBeVisible();
  // Attribution moved to About, so no screen carries a footer landmark any more.
  await expect(page.getByRole("contentinfo")).toHaveCount(0);
});

test("the attribution list holds together at the minimum window size", async ({ page }) => {
  await fixture(page);
  await page.goto("/#/about");
  await expect(page.getByRole("heading", { name: "SageDock", exact: true })).toBeVisible();
  await page.screenshot({ path: "docs/qa/about-light.png", fullPage: true });

  // The credit rows carry long package lists ("NumPy, SciPy, pandas, scikit-learn,
  // matplotlib"), so the narrowest supported window is where they would overflow if the
  // layout were wrong. No other test covers the About page at this width.
  await page.setViewportSize({ width: 860, height: 560 });
  await expect(page.locator(".credit-row").first()).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  await page.screenshot({ path: "docs/qa/about-minimum-light.png", fullPage: true });
});

test("Home opens the JupyterLab launcher, not a notebook or a course folder", async ({ page }) => {
  await fixture(page, { running: false });
  await page.goto("/");
  const launch = page.getByRole("button", { name: "Open JupyterLab", exact: true });
  await expect(launch).toHaveClass(/btn-accent/);
  await expect(page.getByRole("button", { name: "New Sage Notebook" })).not.toHaveClass(
    /btn-accent/,
  );
  await expect(launch).toBeInViewport();
  await launch.click();

  // A dedicated command, not `open_notebook("")`. That distinction is the whole point: an
  // empty notebook path addresses the file browser at whichever course was launched last,
  // while this opens Lab's own landing page rooted at the default SageDock folder.
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).qaRequests.filter((r: any) => r.command === "open_jupyter_home"),
      ),
    )
    .toEqual([{ command: "open_jupyter_home", args: {} }]);
  const calls = await page.evaluate(() => (window as any).qaCalls as string[]);
  expect(calls).not.toContain("open_notebook");
  expect(calls).not.toContain("new_notebook");
  expect(calls).not.toContain("launch_workspace");
  expect(calls).not.toContain("create_workspace");
  await expect(page.getByText("Computing environment running", { exact: true })).toBeVisible();
});

test("a workspace card opens its own folder rather than the launcher", async ({ page }) => {
  await fixture(page, { running: false });
  await page.goto("/");
  await page
    .locator(".workspace-card", { hasText: "Calculus" })
    .getByRole("button", { name: "Launch workspace" })
    .click();

  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).qaRequests.filter((r: any) => r.command === "launch_workspace"),
      ),
    )
    .toEqual([{ command: "launch_workspace", args: { id: "ws-1" } }]);
  const calls = await page.evaluate(() => (window as any).qaCalls as string[]);
  expect(calls).not.toContain("open_jupyter_home");
});

test("JupyterLab startup failure is visible and the main button can retry", async ({ page }) => {
  await fixture(page, { jupyterFails: true });
  await page.goto("/");
  const launch = page.getByRole("button", { name: "Open JupyterLab", exact: true });
  await launch.click();
  await expect(
    page.getByText("Your saved notebooks are safe. Try again.", { exact: true }),
  ).toBeVisible();
  await expect(launch).toBeEnabled();
  await launch.click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as any).qaCalls.filter((c: string) => c === "open_jupyter_home").length,
      ),
    )
    .toBe(2);
});

// --- compilers and build tools ---------------------------------------------------------

/**
 * A tool card, found by its heading rather than by any text it happens to contain.
 *
 * The complete-toolkit card lists every component's label, so filtering cards on text like
 * "Fortran compiler" matches that card as well as the Fortran one.
 */
const card = (page: Page, heading: string) =>
  page
    .locator(".tool-card")
    .filter({ has: page.getByRole("heading", { name: heading, exact: true }) });

test("each build program is reported separately, and a partial group offers repair", async ({
  page,
}) => {
  await fixture(page, { tools: "partial" });
  await page.goto("/#/tools");

  // The state word shares its element with the icon and the version, so the element's text
  // is matched rather than expecting the word to stand alone as its own node.
  const build = card(page, "Build tools");
  await expect(build.locator(".tool-state")).toHaveClass(/is-warn/);
  await expect(build.locator(".tool-state")).toContainText("Needs repair");

  // Three independent programs, each on its own line. A single tick for the group would
  // hide the two that are missing, which is the defect a PATH-only check produces.
  await expect(build.getByText("Make", { exact: true })).toBeVisible();
  await expect(build.getByText("CMake", { exact: true })).toBeVisible();
  await expect(build.getByText("pkg-config", { exact: true })).toBeVisible();
  await expect(build.getByText("Missing")).toHaveCount(2);
  await expect(build.getByRole("button", { name: "Repair", exact: true })).toBeVisible();

  // A complete group reads as installed and offers verification instead of installation.
  const cpp = card(page, "C / C++ compiler");
  await expect(cpp.locator(".tool-state")).toHaveClass(/is-ok/);
  await expect(cpp.getByRole("button", { name: "Verify", exact: true })).toBeVisible();

  // The complete toolkit is derived, so one missing program makes it a repair too.
  await expect(card(page, "Full scientific build toolkit").locator(".tool-state")).toHaveClass(
    /is-warn/,
  );
});

test("Install all installs the whole toolkit in one step", async ({ page }) => {
  await fixture(page, { tools: "none" });
  await page.goto("/#/tools");

  await card(page, "Full scientific build toolkit")
    .getByRole("button", { name: "Install all", exact: true })
    .click();
  await expect(page.getByText(/passed its check/)).toBeVisible();

  for (const heading of ["C / C++ compiler", "Fortran compiler", "Build tools"]) {
    await expect(card(page, heading).locator(".tool-state")).toHaveClass(/is-ok/);
  }

  // One request for the whole kit, not one per component.
  const installs = await page.evaluate(() =>
    (window as any).qaRequests.filter((r: any) => r.command === "install_scientific_tool"),
  );
  expect(installs).toEqual([{ command: "install_scientific_tool", args: { tool: "toolkit" } }]);
});

test("a failed verification is explained and the retry succeeds", async ({ page }) => {
  await fixture(page, { tools: "none", installFailsOnce: true });
  await page.goto("/#/tools");

  const fortran = card(page, "Fortran compiler");
  await expect(fortran.locator(".tool-state")).toHaveClass(/is-off/);

  await fortran.getByRole("button", { name: "Install", exact: true }).click();
  await expect(page.getByText(/didn't pass its check/)).toBeVisible();
  // Installing is not the same as working: the card must not claim success.
  await expect(fortran.locator(".tool-state")).not.toHaveClass(/is-ok/);

  await fortran.getByRole("button", { name: "Install", exact: true }).click();
  await expect(fortran.locator(".tool-state")).toHaveClass(/is-ok/);
});

test("an unavailable status is never shown as 'not installed'", async ({ page }) => {
  await fixture(page, { tools: "unavailable", running: false });
  await page.goto("/#/tools");

  await expect(page.getByText("SageDock hasn't checked these yet")).toBeVisible();
  const cpp = card(page, "C / C++ compiler");
  await expect(cpp.locator(".tool-state")).toContainText("Status unavailable");
  await expect(cpp.locator(".tool-state")).not.toContainText("Not installed");

  // Checking is the one action allowed to start a stopped environment, and only on request.
  // The tools were there all along; SageDock simply had not looked.
  await cpp.getByRole("button", { name: "Check now", exact: true }).click();
  await expect(cpp.locator(".tool-state")).toHaveClass(/is-ok/);
  const probes = await page.evaluate(() =>
    (window as any).qaRequests.filter((r: any) => r.command === "scientific_tools"),
  );
  expect(probes.some((r: any) => r.args.force === true)).toBe(true);
});

test("Home shows unknown tool status and never wakes a stopped environment", async ({ page }) => {
  await fixture(page, { running: false, tools: "unavailable" });
  await page.goto("/");

  const section = page.getByRole("region", { name: "Compilers and build tools" });
  await expect(section.getByText("Status unknown").first()).toBeVisible();
  // The distinction this section exists to preserve.
  await expect(section.getByText("Not installed")).toHaveCount(0);
  await expect(page.getByText("Computing environment stopped")).toBeVisible();

  const probes = await page.evaluate(() =>
    (window as any).qaRequests.filter((r: any) => r.command === "scientific_tools"),
  );
  expect(probes.length).toBeGreaterThan(0);
  expect(probes.every((r: any) => r.args.force === false)).toBe(true);
  const calls = await page.evaluate(() => (window as any).qaCalls as string[]);
  expect(calls).not.toContain("open_jupyter_home");
  expect(calls).not.toContain("launch_workspace");
});

test("Home reflects a compiler becoming available after it is installed", async ({ page }) => {
  await fixture(page, { tools: "none" });
  await page.goto("/");
  await expect(
    page.getByRole("region", { name: "Compilers and build tools" }).locator(".capability.is-off"),
  ).toHaveCount(3);

  await page.getByRole("link", { name: "Scientific tools", exact: false }).click();
  await page
    .locator(".tool-card", { hasText: "Fortran compiler" })
    .getByRole("button", { name: "Install", exact: true })
    .click();
  await expect(page.getByText(/passed its check/)).toBeVisible();

  await page.getByRole("link", { name: "Workspace", exact: false }).first().click();
  await expect(
    page.getByRole("region", { name: "Compilers and build tools" }).locator(".capability.is-ok"),
  ).toHaveCount(1);
});

test("compilers are never advertised as notebook languages", async ({ page }) => {
  await fixture(page);
  await page.goto("/");
  await expect(
    page
      .getByRole("region", { name: "Compilers and build tools" })
      .getByText(/do not add new notebook languages/),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: /New C\+\+ Notebook/ })).toHaveCount(0);
  await expect(page.getByRole("button", { name: /New Fortran Notebook/ })).toHaveCount(0);

  await page.goto("/#/tools");
  await expect(page.getByText("Compilers are not notebook languages")).toBeVisible();
});

test("the tools page keeps distinct icons and holds at the minimum size in both themes", async ({
  page,
}) => {
  await fixture(page, { tools: "partial" });
  await page.goto("/#/tools");
  await expect(page.getByRole("heading", { name: "C / C++ compiler" })).toBeVisible();

  // Four compiler cards, four different glyphs: the shared generic plus icon is gone.
  const glyphs = await page
    .locator("section[aria-label='Compilers and build tools'] .tool-icon .icon")
    .allTextContents();
  expect(glyphs).toHaveLength(4);
  expect(new Set(glyphs).size).toBe(4);

  await page.screenshot({ path: "docs/qa/tools-light.png", fullPage: true });
  await page.setViewportSize({ width: 860, height: 560 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  await page.screenshot({ path: "docs/qa/tools-minimum-light.png", fullPage: true });

  await page.getByRole("link", { name: "Settings", exact: false }).first().click();
  await page.getByRole("button", { name: "Dark", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.getByRole("link", { name: "Scientific tools", exact: false }).click();
  await expect(page.getByRole("heading", { name: "C / C++ compiler" })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(
    true,
  );
  await page.screenshot({ path: "docs/qa/tools-dark.png", fullPage: true });
});

// --- renaming, adding files, and the two notebook lists -------------------------------

test("a workspace is renamed from the pencil beside its name", async ({ page }) => {
  await fixture(page);
  await page.goto("/");

  const physics = page.locator(".workspace-card", { hasText: "Physics" });
  // The pencil lives next to the title, so it travels with the name rather than sitting in
  // the action row. The row now offers adding files instead.
  await expect(physics.locator(".ws-title .ws-edit")).toBeVisible();
  await expect(physics.getByRole("button", { name: "Rename Physics" })).toBeVisible();
  await expect(physics.getByRole("button", { name: "Add files to Physics" })).toBeVisible();

  await physics.getByRole("button", { name: "Rename Physics" }).click();
  await page.getByLabel("New name for this workspace").fill("Physics 101");
  await page.getByRole("button", { name: "Save name", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Physics 101" })).toBeVisible();
  // The pencil follows the new name rather than staying attached to the old label.
  await expect(
    page
      .locator(".workspace-card", { hasText: "Physics 101" })
      .getByRole("button", { name: "Rename Physics 101" }),
  ).toBeVisible();
});

test("renaming is refused with a pop-up while a notebook is open in that workspace", async ({
  page,
}) => {
  await fixture(page, { renameBlocked: true });
  await page.goto("/");

  await page
    .locator(".workspace-card", { hasText: "Physics" })
    .getByRole("button", { name: "Rename Physics" })
    .click();
  await page.getByLabel("New name for this workspace").fill("Physics 101");
  await page.getByRole("button", { name: "Save name", exact: true }).click();

  // A pop-up, not a banner: it interrupts something the user just asked for.
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("SageMath is using this workspace")).toBeVisible();
  // The backend's own wording is shown verbatim rather than paraphrased, so the reason
  // the user reads is the reason the backend actually gave.
  await expect(dialog.getByText(/Renaming moves the folder on disk/)).toBeVisible();
  await expect(dialog.getByText(/Nothing has been changed/)).toBeVisible();
  await expect(dialog.getByText(/Save your open notebooks/)).toBeVisible();
  // Nothing to decide, so one button closes it rather than offering two words for leaving.
  await expect(dialog.getByRole("button", { name: "Cancel", exact: true })).toHaveCount(0);
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  // The old name is intact, because the rename never happened.
  await expect(page.getByRole("heading", { name: "Physics" })).toBeVisible();
});

test("files can be added to a workspace from its own card", async ({ page }) => {
  await fixture(page);
  await page.goto("/");

  await page
    .locator(".workspace-card", { hasText: "Calculus" })
    .getByRole("button", { name: "Add files to Calculus" })
    .click();
  await expect(page.getByText(/2 file\(s\) copied into Calculus/)).toBeVisible();
  await expect(page.getByText(/Nothing already there was replaced/)).toBeVisible();

  const requests = await page.evaluate(() =>
    (window as any).qaRequests.filter((r: any) => r.command === "add_files_to_workspace"),
  );
  expect(requests).toEqual([{ command: "add_files_to_workspace", args: { id: "ws-1" } }]);
});

test("Home lists recently opened and recently downloaded notebooks separately", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/");

  await expect(page.getByRole("heading", { name: /Recently Opened/ })).toBeVisible();
  await expect(page.getByRole("heading", { name: /Recent notebooks/ })).toHaveCount(0);

  const downloadsSection = page.getByRole("region", { name: "Downloads" });
  await expect(downloadsSection.getByText("Assignment 1")).toBeVisible();
  // A notebook the browser saved as JSON is listed, under the name the student recognises
  // rather than its extension chain.
  await expect(downloadsSection.getByText("Week 02 Lab")).toBeVisible();
  await expect(downloadsSection.getByText("Week 02 Lab.ipynb.json")).toHaveCount(0);
  // Downloads are not in a workspace yet; that explanation now lives behind the info icon
  // rather than as a permanent paragraph.
  await expect(downloadsSection.getByRole("button", { name: "About this list" })).toHaveAttribute(
    "title",
    /Read when SageDock opens/,
  );
});

test("clicking a downloaded notebook offers a choice of workspace, and a folder that isn't available can't be chosen", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/");

  await page
    .getByRole("region", { name: "Downloads" })
    .getByRole("button", { name: /Assignment 1/ })
    .click();

  const picker = page.getByRole("dialog");
  await expect(picker).toBeVisible();
  await expect(picker.getByText("Add to which workspace?")).toBeVisible();
  await expect(picker.locator(".list-row", { hasText: "Calculus" })).toBeEnabled();
  await expect(picker.locator(".list-row", { hasText: "Physics" })).toBeEnabled();
  // Listed rather than hidden, so a student can see the workspace exists even though it
  // can't be used from here right now — the same choice WorkspaceCard makes for Launch.
  const missing = picker.locator(".list-row", { hasText: "Old Laptop Folder" });
  await expect(missing).toBeDisabled();
  await expect(missing).toContainText("isn't available right now");

  await picker.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  const calls = await page.evaluate(() => (window as any).qaCalls as string[]);
  expect(calls).not.toContain("check_download_target");
  expect(calls).not.toContain("open_downloaded_notebook");
});

test("choosing a workspace with no conflict copies the download in, opens it, and makes that workspace active", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/");

  await page
    .getByRole("region", { name: "Downloads" })
    .getByRole("button", { name: /Assignment 1/ })
    .click();
  await page.getByRole("dialog").locator(".list-row", { hasText: "Physics" }).click();

  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).qaRequests.filter(
          (r: any) =>
            r.command === "check_download_target" || r.command === "open_downloaded_notebook",
        ),
      ),
    )
    .toEqual([
      {
        command: "check_download_target",
        args: { name: "Assignment 1.ipynb", workspaceId: "ws-2" },
      },
      {
        command: "open_downloaded_notebook",
        args: { name: "Assignment 1.ipynb", workspaceId: "ws-2", onConflict: "copy" },
      },
    ]);
  await expect(page.getByText("Assignment 1.ipynb is open in Physics.")).toBeVisible();
  // Opening a workspace from a download makes it active, the same as its own card's
  // Launch button — Home must not quietly leave Calculus marked as the one in use.
  await expect(
    page.locator(".workspace-card", { hasText: "Physics" }).getByText("In use"),
  ).toBeVisible();
  await expect(
    page.locator(".workspace-card", { hasText: "Calculus" }).getByText("In use"),
  ).toHaveCount(0);
});

test("a name already in Windows offers to open the existing notebook, keeping it untouched", async ({
  page,
}) => {
  await fixture(page, { existingWorkspaceFiles: { "ws-1": ["Assignment 1.ipynb"] } });
  await page.goto("/");

  await page
    .getByRole("region", { name: "Downloads" })
    .getByRole("button", { name: /Assignment 1/ })
    .click();
  await page.getByRole("dialog").locator(".list-row", { hasText: "Calculus" }).click();

  const conflict = page.getByRole("dialog");
  await expect(conflict.getByText("This notebook already exists")).toBeVisible();
  await expect(conflict.getByText("Assignment 1.ipynb")).toBeVisible();
  await expect(conflict.getByText("Calculus")).toBeVisible();

  await conflict.locator(".list-row", { hasText: "Open the existing notebook" }).click();

  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).qaRequests.filter((r: any) => r.command === "open_downloaded_notebook"),
      ),
    )
    .toEqual([
      {
        command: "open_downloaded_notebook",
        args: { name: "Assignment 1.ipynb", workspaceId: "ws-1", onConflict: "open_existing" },
      },
    ]);
  await expect(page.getByText("Assignment 1.ipynb is open in Calculus.")).toBeVisible();
});

test("replacing a colliding download overwrites the existing notebook under the same name", async ({
  page,
}) => {
  await fixture(page, { existingWorkspaceFiles: { "ws-1": ["Assignment 1.ipynb"] } });
  await page.goto("/");

  await page
    .getByRole("region", { name: "Downloads" })
    .getByRole("button", { name: /Assignment 1/ })
    .click();
  await page.getByRole("dialog").locator(".list-row", { hasText: "Calculus" }).click();

  const conflict = page.getByRole("dialog");
  const replace = conflict.locator(".list-row", { hasText: "Replace it" });
  await expect(replace).toHaveClass(/is-danger/);
  await expect(replace).toContainText("can't be undone");
  await replace.click();

  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).qaRequests.filter((r: any) => r.command === "open_downloaded_notebook"),
      ),
    )
    .toEqual([
      {
        command: "open_downloaded_notebook",
        args: { name: "Assignment 1.ipynb", workspaceId: "ws-1", onConflict: "replace" },
      },
    ]);
  await expect(page.getByText("Assignment 1.ipynb is open in Calculus.")).toBeVisible();
});

test("making another copy of a colliding download keeps both notebooks under different names", async ({
  page,
}) => {
  await fixture(page, { existingWorkspaceFiles: { "ws-1": ["Assignment 1.ipynb"] } });
  await page.goto("/");

  await page
    .getByRole("region", { name: "Downloads" })
    .getByRole("button", { name: /Assignment 1/ })
    .click();
  await page.getByRole("dialog").locator(".list-row", { hasText: "Calculus" }).click();

  await page.getByRole("dialog").locator(".list-row", { hasText: "Make another copy" }).click();

  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).qaRequests.filter((r: any) => r.command === "open_downloaded_notebook"),
      ),
    )
    .toEqual([
      {
        command: "open_downloaded_notebook",
        args: { name: "Assignment 1.ipynb", workspaceId: "ws-1", onConflict: "copy" },
      },
    ]);
  // The dedup name, not the colliding one — proof the existing file was left alone.
  await expect(page.getByText("Assignment 1 (1).ipynb is open in Calculus.")).toBeVisible();
});

test("cancelling the name-collision dialog leaves the existing notebook untouched", async ({
  page,
}) => {
  await fixture(page, { existingWorkspaceFiles: { "ws-1": ["Assignment 1.ipynb"] } });
  await page.goto("/");

  await page
    .getByRole("region", { name: "Downloads" })
    .getByRole("button", { name: /Assignment 1/ })
    .click();
  await page.getByRole("dialog").locator(".list-row", { hasText: "Calculus" }).click();

  const conflict = page.getByRole("dialog");
  await expect(conflict.getByText("This notebook already exists")).toBeVisible();
  await conflict.getByRole("button", { name: "Cancel", exact: true }).click();

  await expect(page.getByRole("dialog")).toHaveCount(0);
  const calls = await page.evaluate(() => (window as any).qaCalls as string[]);
  expect(calls).not.toContain("open_downloaded_notebook");
});

test("a notebook can be dragged out of SageDock to Windows", async ({ page }) => {
  await fixture(page);
  await page.goto("/");

  await page.getByRole("button", { name: /Exploring prime numbers/ }).dispatchEvent("dragstart");
  await page
    .getByRole("region", { name: "Downloads" })
    .getByRole("button", { name: /Assignment 1/ })
    .dispatchEvent("dragstart");

  // The backend is told which list the name came from; it resolves the real path itself,
  // so no absolute path is ever sent from the page.
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).qaRequests.filter((r: any) => r.command === "start_notebook_drag"),
      ),
    )
    .toEqual([
      {
        command: "start_notebook_drag",
        args: { path: "Notebooks/Exploring prime numbers.ipynb", source: "workspace" },
      },
      {
        command: "start_notebook_drag",
        args: { path: "Assignment 1.ipynb", source: "downloads" },
      },
    ]);
});

test("an empty Downloads folder explains itself instead of showing a bare list", async ({
  page,
}) => {
  await fixture(page, { downloads: [] });
  await page.goto("/");

  const downloadsSection = page.getByRole("region", { name: "Downloads" });
  await expect(downloadsSection.getByText("No downloaded notebooks yet")).toBeVisible();
});

test("the Downloads list can be refreshed without reloading the rest of Home", async ({ page }) => {
  await fixture(page);
  await page.goto("/");

  const section = page.getByRole("region", { name: "Downloads" });
  const reads = () =>
    page.evaluate(
      () =>
        (window as any).qaRequests.filter((r: any) => r.command === "downloaded_notebooks").length,
    );
  const before = await reads();

  await section.getByRole("button", { name: "Refresh", exact: true }).click();

  // The folder is read again on demand, because a browser can write into it while Home is
  // already open and nothing would otherwise tell SageDock that happened.
  await expect.poll(reads).toBeGreaterThan(before);
  await expect(page.getByText(/notebook\(s\) in your Downloads folder/)).toBeVisible();
  // Refreshing this one list must not re-run setup or wake the environment.
  const calls = await page.evaluate(() => (window as any).qaCalls as string[]);
  expect(calls).not.toContain("run_setup");
  expect(calls).not.toContain("open_jupyter_home");
});

// --- the merged environment card, capped and actionable notebook lists, and notices -----

test("the environment status and JupyterLab launcher are one card, not two", async ({ page }) => {
  await fixture(page);
  await page.goto("/");

  // The two used to be separate sections; now there is exactly one, and it carries both
  // the status and both actions that act on it.
  const card = page.getByRole("region", { name: "Computing environment" });
  await expect(card).toBeVisible();
  await expect(page.getByRole("heading", { name: "Start working" })).toHaveCount(0);
  await expect(card.getByText("Computing environment running", { exact: true })).toBeVisible();
  await expect(card.getByRole("button", { name: "Open JupyterLab", exact: true })).toBeVisible();
  await expect(card.getByRole("button", { name: "Stop SageMath", exact: true })).toBeVisible();

  // The long explanation is behind the info icon now, not sitting permanently on the page.
  await expect(page.getByText(/Stopping it frees that memory/)).toHaveCount(0);
  await expect(card.getByRole("button", { name: "What this means" })).toHaveAttribute(
    "title",
    /Stopping it frees that memory/,
  );
});

test("Recently Opened caps at six by default, and search still reaches the rest", async ({
  page,
}) => {
  const recent = Array.from({ length: 8 }, (_, i) => ({
    name: `Notebook ${i + 1}`,
    path: `Notebooks/Notebook ${i + 1}.ipynb`,
    modified: 1789473600 - i * 3600,
  }));
  await fixture(page, { recent });
  await page.goto("/");

  const section = page.getByRole("region", { name: "Recently Opened" });
  // The count badge still reports the true total, only the rows shown are capped.
  await expect(section.getByText("8", { exact: true })).toBeVisible();
  await expect(section.locator(".notebook-row")).toHaveCount(6);
  await expect(section.getByRole("button", { name: /Notebook 1\b/ })).toBeVisible();
  await expect(section.getByRole("button", { name: /Notebook 7/ })).toHaveCount(0);

  await page.getByLabel("Find a notebook").fill("Notebook 7");
  await expect(section.getByRole("button", { name: /Notebook 7/ })).toBeVisible();
});

test("hovering a Recently Opened row swaps the date for Delete, Show in File Explorer, and Open", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/");

  const row = page
    .getByRole("region", { name: "Recently Opened" })
    .locator(".notebook-row", { hasText: "Exploring prime numbers" });
  await expect(row.locator(".notebook-row-date")).toBeVisible();
  await expect(row.getByRole("button", { name: /Delete/ })).toBeHidden();

  await row.hover();
  await expect(row.getByRole("button", { name: "Delete Exploring prime numbers" })).toBeVisible();
  await expect(
    row.getByRole("button", { name: "Show Exploring prime numbers in File Explorer" }),
  ).toBeVisible();
  await expect(row.getByRole("button", { name: "Open Exploring prime numbers" })).toBeVisible();
});

test("deleting a recent notebook asks for confirmation and moves it to the Recycle Bin", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/");

  const section = page.getByRole("region", { name: "Recently Opened" });
  const row = section.locator(".notebook-row", { hasText: "Exploring prime numbers" });
  await row.hover();
  await row.getByRole("button", { name: "Delete Exploring prime numbers" }).click();

  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Delete Exploring prime numbers?")).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "Move to Recycle Bin", exact: true }),
  ).toBeVisible();
  await dialog.getByRole("button", { name: "Move to Recycle Bin", exact: true }).click();

  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).qaRequests.filter((r: any) => r.command === "delete_recent_notebook"),
      ),
    )
    .toEqual([
      {
        command: "delete_recent_notebook",
        args: { path: "Notebooks/Exploring prime numbers.ipynb" },
      },
    ]);
  await expect(page.getByText("was moved to the Recycle Bin")).toBeVisible();
  await expect(section.getByText("Exploring prime numbers")).toHaveCount(0);
});

test("the Downloads ribbon offers Delete, Add to workspace, and Show in File Explorer", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/");

  const row = page
    .getByRole("region", { name: "Downloads" })
    .locator(".notebook-row", { hasText: "Assignment 1" });
  await row.hover();

  // There is deliberately no Open action on the ribbon. Clicking the row already opens the
  // file, so a second control doing the same thing was redundant. Asserted rather than
  // simply dropped, so putting it back is a deliberate act and not an accident.
  await expect(row.getByRole("button", { name: "Open Assignment 1" })).toHaveCount(0);

  await row.getByRole("button", { name: "Show Assignment 1 in File Explorer" }).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).qaRequests.filter((r: any) => r.command === "reveal_downloaded_file"),
      ),
    )
    .toEqual([{ command: "reveal_downloaded_file", args: { name: "Assignment 1.ipynb" } }]);

  await row.hover();
  await row.getByRole("button", { name: "Add Assignment 1 to a workspace" }).click();
  await expect(page.getByRole("dialog").getByText("Add to which workspace?")).toBeVisible();
});

test("deleting a download asks for confirmation and moves it to the Recycle Bin", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/");

  const section = page.getByRole("region", { name: "Downloads" });
  const row = section.locator(".notebook-row", { hasText: "Assignment 1" });
  await row.hover();
  await row.getByRole("button", { name: "Delete Assignment 1" }).click();

  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText("Delete Assignment 1?")).toBeVisible();
  await dialog.getByRole("button", { name: "Move to Recycle Bin", exact: true }).click();

  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).qaRequests.filter((r: any) => r.command === "delete_downloaded_file"),
      ),
    )
    .toEqual([{ command: "delete_downloaded_file", args: { name: "Assignment 1.ipynb" } }]);
  await expect(section.getByText("Assignment 1", { exact: true })).toHaveCount(0);
});

test("Open notebook goes through the same workspace-and-conflict flow as a download", async ({
  page,
}) => {
  await fixture(page, {
    pickedFile: "Physics Homework.ipynb",
    existingWorkspaceFiles: { "ws-1": ["Physics Homework.ipynb"] },
  });
  await page.goto("/");

  await page.getByRole("button", { name: "Open notebook", exact: true }).click();
  const picker = page.getByRole("dialog");
  await expect(picker.getByText("Add to which workspace?")).toBeVisible();
  await expect(picker.getByText("Physics Homework.ipynb")).toBeVisible();

  await picker.locator(".list-row", { hasText: "Calculus" }).click();
  const conflict = page.getByRole("dialog");
  await expect(conflict.getByText("This notebook already exists")).toBeVisible();
  await conflict.locator(".list-row", { hasText: "Make another copy" }).click();

  await expect
    .poll(() =>
      page.evaluate(() =>
        (window as any).qaRequests.filter((r: any) => r.command === "open_picked_notebook"),
      ),
    )
    .toEqual([
      { command: "open_picked_notebook", args: { workspaceId: "ws-1", onConflict: "copy" } },
    ]);
  await expect(page.getByText("Physics Homework (1).ipynb is open in Calculus.")).toBeVisible();
});

test("closing the Open notebook picker without choosing a file starts no workspace flow", async ({
  page,
}) => {
  await fixture(page, { pickedFile: null });
  await page.goto("/");

  await page.getByRole("button", { name: "Open notebook", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  const calls = await page.evaluate(() => (window as any).qaCalls as string[]);
  expect(calls).not.toContain("check_picked_target");
});

test("a success notice clears itself after five seconds", async ({ page }) => {
  await fixture(page);
  await page.goto("/");

  await page
    .locator(".workspace-card", { hasText: "Calculus" })
    .getByRole("button", { name: "Add files to Calculus" })
    .click();
  const notice = page.getByText(/copied into Calculus/);
  await expect(notice).toBeVisible();
  await expect(notice).toBeHidden({ timeout: 6000 });
});

test("a newly downloaded notebook is announced, and the notice auto-dismisses", async ({
  page,
}) => {
  // Real time, not mocked: it exercises the actual 8-second poll interval and the actual
  // 5-second auto-dismiss timer end to end, which needs more room than the default budget.
  test.setTimeout(45000);
  await fixture(page);
  await page.goto("/");

  // Let the poll's first tick seed itself on what's already there before anything new
  // appears, the same as a real session that opened before the download happened.
  await page.waitForTimeout(8500);
  await page.evaluate(() => {
    (window as any).qaAddDownload({ name: "Lab 3", path: "Lab 3.ipynb", modified: 1789646400 });
  });
  await expect(page.getByText("Lab 3 was downloaded. Find it in Downloads.")).toBeVisible({
    timeout: 9000,
  });
  await expect(page.getByText("Lab 3 was downloaded. Find it in Downloads.")).toBeHidden({
    timeout: 6000,
  });
});

test("a failed tool-status request leaves an actionable unknown status", async ({ page }) => {
  await fixture(page);
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Open JupyterLab", exact: true })).toBeVisible();
  await page.evaluate(() => {
    const api = (window as any).__TAURI_INTERNALS__;
    const original = api.invoke;
    api.invoke = (command: string, args: unknown) => {
      if (command === "scientific_tools")
        return Promise.reject({
          title: "Tool status unavailable",
          message: "Retry the tool check. Your notebooks are safe.",
        });
      return original(command, args);
    };
  });
  await page.getByRole("link", { name: "Scientific tools", exact: false }).click();
  const cpp = card(page, "C / C++ compiler");
  await expect(cpp.locator(".tool-state")).toContainText("Status unavailable");
  await expect(cpp.getByRole("button", { name: "Check now", exact: true })).toBeEnabled();
  await cpp.getByRole("button", { name: "Check now", exact: true }).click();
  await expect(cpp.locator(".tool-state")).not.toContainText("Installed");
  await expect(cpp.getByRole("button", { name: "Check now", exact: true })).toBeEnabled();
});

test("a first run explains the app before asking for anything", async ({ page }) => {
  await fixture(page, { showOnboarding: true });
  await page.goto("/");

  // The app itself must not be reachable behind the introduction.
  await expect(page.getByRole("button", { name: "New Sage Notebook" })).toHaveCount(0);
  await expect(page.getByRole("heading", { name: /without a terminal/ })).toBeVisible();
  await expect(page.getByText("Step 1 of 4")).toBeVisible();
  await page.screenshot({ path: "docs/qa/onboarding-light.png", fullPage: true });

  // The explanatory steps come first; nothing has been changed yet.
  await page.getByRole("button", { name: "Next" }).click();
  await expect(page.getByRole("heading", { name: /ordinary Windows folders/ })).toBeVisible();
  await page.getByRole("button", { name: "Next" }).click();
  await expect(page.getByRole("heading", { name: /downloads, and backups/ })).toBeVisible();
  expect(
    await page.evaluate(() => (window as any).qaCalls.includes("set_onboarding_complete")),
  ).toBe(false);

  await page.getByRole("button", { name: "Next" }).click();
  await expect(page.getByText("Step 4 of 4")).toBeVisible();
  await expect(page.getByRole("group", { name: "Appearance" })).toBeVisible();

  // The browser picker is meaningless until notebooks open in a browser, so it only
  // appears once that is chosen.
  await expect(page.getByLabel(/Which browser/)).toHaveCount(0);
  await page
    .getByRole("group", { name: "Where notebooks open" })
    .getByRole("button", { name: "Browser", exact: true })
    .click();
  const picker = page.getByLabel(/Which browser/);
  await expect(picker).toBeVisible();
  await picker.selectOption("Firefox-308046B0AF4A39CB");

  await page.getByRole("button", { name: "Start using SageDock" }).click();
  await expect(page.getByRole("button", { name: "New Sage Notebook" })).toBeVisible();

  const requests = await page.evaluate(() => (window as any).qaRequests);
  expect(requests.find((r: any) => r.command === "set_preferred_browser")?.args.value).toBe(
    "Firefox-308046B0AF4A39CB",
  );
  expect(requests.find((r: any) => r.command === "set_onboarding_complete")?.args.value).toBe(true);
});

test("skipping the introduction still counts as having seen it", async ({ page }) => {
  await fixture(page, { showOnboarding: true });
  await page.goto("/");

  await page.getByRole("button", { name: "Skip introduction" }).click();

  await expect(page.getByRole("button", { name: "New Sage Notebook" })).toBeVisible();
  const requests = await page.evaluate(() => (window as any).qaRequests);
  expect(requests.find((r: any) => r.command === "set_onboarding_complete")?.args.value).toBe(true);
});

test("an installation that has seen the introduction opens straight into the app", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/");

  await expect(page.getByRole("button", { name: "New Sage Notebook" })).toBeVisible();
  await expect(page.getByText(/Step 1 of/)).toHaveCount(0);
});

test("Settings can bring the introduction back, and opens the workspaces folder", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/settings");

  // The folder row is about the installation, not whichever course is selected.
  await expect(page.getByRole("heading", { name: "Workspaces folder" })).toBeVisible();
  await page.getByRole("button", { name: "Open folder" }).click();
  await expect
    .poll(() => page.evaluate(() => (window as any).qaCalls.includes("open_workspaces_folder")))
    .toBe(true);

  await page.getByRole("button", { name: "Show again" }).click();
  const requests = await page.evaluate(() => (window as any).qaRequests);
  expect(requests.find((r: any) => r.command === "set_onboarding_complete")?.args.value).toBe(
    false,
  );
});

test("the Settings browser picker appears only when notebooks open in a browser", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/#/settings");

  await expect(page.getByLabel("Which browser")).toHaveCount(0);
  await page
    .getByRole("group", { name: "Notebook destination" })
    .getByRole("button", { name: "Browser", exact: true })
    .click();

  const picker = page.getByLabel("Which browser");
  await expect(picker).toBeVisible();
  await expect(picker.getByRole("option", { name: "Google Chrome" })).toHaveCount(1);
  await expect(picker.getByRole("option", { name: "Your default browser" })).toHaveCount(1);
});

test("a download saved outside the Downloads folder says which folder it went to", async ({
  page,
}) => {
  await fixture(page, {
    downloads: [
      { name: "Assignment 1", path: "Assignment 1.ipynb", modified: 1789473600 },
      {
        name: "Lecture slides.pdf",
        path: "tracked:9f86d081884c7d65",
        modified: 1789387200,
        folder: "Desktop",
        notebook: false,
      },
    ],
  });
  await page.goto("/");

  const tracked = page.locator(".notebook-row", { hasText: "Lecture slides.pdf" });
  await expect(tracked).toBeVisible();

  // The row names the folder it landed in. It must never show a full path: the backend
  // deliberately sends only the folder name.
  await expect(tracked.getByText("Desktop", { exact: true })).toBeVisible();
  await expect(tracked).not.toContainText("C:");

  // The action buttons are `display: none` until the row is hovered, so they are absent
  // from the accessibility tree until then. Asserting on them without hovering would pass
  // whether a button were genuinely missing or merely hidden, and so prove nothing.
  await tracked.hover();
  await expect(
    tracked.getByRole("button", { name: "Show Lecture slides.pdf in File Explorer" }),
  ).toBeVisible();
  await expect(tracked.getByRole("button", { name: "Delete Lecture slides.pdf" })).toBeVisible();
  // A PDF has no workspace to add it to, so that one action is gone while the rest remain.
  await expect(tracked.getByRole("button", { name: /Add .* to a workspace/ })).toHaveCount(0);

  // An ordinary notebook in Downloads still offers it, and reads as "Downloads".
  const notebook = page.locator(".notebook-row", { hasText: "Assignment 1" });
  await expect(notebook.getByText("Downloads", { exact: true })).toBeVisible();
  await notebook.hover();
  await expect(
    notebook.getByRole("button", { name: "Add Assignment 1 to a workspace" }),
  ).toBeVisible();

  // Opening a non-notebook hands it to Windows rather than starting the workspace flow.
  await tracked.locator(".list-row-open").click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).qaCalls.includes("open_downloaded_file_externally")),
    )
    .toBe(true);
  await expect(page.getByRole("dialog")).toHaveCount(0);
});
