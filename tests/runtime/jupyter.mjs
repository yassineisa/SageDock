// Invoked by the isolated Rust integration test. Authentication stays in stdin, not logs.
import { chromium, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
const input = JSON.parse(readFileSync(0, "utf8"));
const watchdog = setTimeout(() => process.exit(2), 150000);
let browser;
try {
  browser = await chromium.launch({ channel: "msedge" });
  const page = await browser.newPage();
  page.setDefaultTimeout(30000);
  const dialogs = [];
  page.on("dialog", async (dialog) => {
    dialogs.push(dialog.message());
    await dialog.dismiss();
  });
  await page.goto(input.url);
  await page.evaluate(input.home);
  await expect(page.locator(".jp-Launcher").last()).toBeVisible();
  await page.evaluate(input.first);
  await expect
    .poll(() => page.evaluate(() => window.jupyterapp.shell.currentWidget?.context?.path))
    .toBe(input.firstPath);
  await page.evaluate(() => {
    const widget = window.jupyterapp.shell.currentWidget;
    window.qaNotebook = widget;
    window.qaPage = "still here";
    widget.context.model.sharedModel.cells[0].setSource("unsaved audit marker");
  });
  await page.evaluate(input.second);
  await expect
    .poll(() => page.evaluate(() => window.jupyterapp.shell.currentWidget?.context?.path))
    .toBe(input.secondPath);
  await page.evaluate(input.home);
  await expect(page.locator(".jp-Launcher").last()).toBeVisible();
  expect(await page.evaluate(() => window.qaPage)).toBe("still here");
  expect(
    await page.evaluate(() => window.qaNotebook.context.model.sharedModel.cells[0].getSource()),
  ).toBe("unsaved audit marker");
  expect(await page.evaluate(() => window.qaNotebook.isDisposed)).toBe(false);
  // Repeated Home actions reuse the launcher instead of creating unbounded tabs.
  const count = await page.locator(".jp-Launcher").count();
  await page.evaluate(input.home);
  expect(await page.locator(".jp-Launcher").count()).toBe(count);
  // Browser mode uses a fresh named UI workspace and must show the real launcher too.
  const tab = await browser.newPage();
  const url = new URL(input.url);
  url.pathname = "/lab/workspaces/sagedock-audit-browser";
  await tab.goto(url.toString());
  await expect(tab.locator(".jp-Launcher")).toBeVisible({ timeout: 60000 });
  expect(dialogs).toEqual([]);
  console.log("REAL JUPYTER LAUNCHER AND UNSAVED EDITS VERIFIED");
} catch (error) {
  console.error(
    String(error).replaceAll(new URL(input.url).searchParams.get("token"), "[redacted]"),
  );
  process.exitCode = 1;
} finally {
  await browser?.close();
  clearTimeout(watchdog);
}
