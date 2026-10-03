// SPDX-License-Identifier: Apache-2.0
// The app shell on the mocked core: navigation, ⌘K, a scripted recording,
// language and theme, the compact window.
import { expect, test, type Page } from "@playwright/test";

// Windows platform: Ctrl chords are the same on every test OS.
const open = (page: Page, hash = "/meetings") => page.goto(`/?platform=win#${hash}`);

test("navigates with the sidebar and the command palette", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("heading", { name: "Meetings" })).toBeVisible();
  const nav = page.getByRole("navigation", { name: "Main" });
  await nav.getByRole("link", { name: "People" }).click();
  await expect(page.getByRole("heading", { name: "People" })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Live" })).toHaveCount(0);

  await page.keyboard.press("Control+K");
  const palette = page.getByRole("dialog", { name: "Command palette" });
  await expect(palette).toBeVisible();
  await page.keyboard.type("set");
  await expect(palette.getByRole("option").first()).toHaveText(/Ask “set”/);
  await palette.getByRole("option", { name: "Settings" }).click();
  await expect(page.getByRole("heading", { name: "General" })).toBeVisible();
});

test("records a meeting on the mocked core", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Record call", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "Meeting title" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: "Live" })).toBeVisible();
  await expect(page.getByRole("main").locator("ol > li").first()).toContainText("Okay, bắt đầu nhé.", { timeout: 5000 });
  // A new speaker turn is announced once ("Me: …"), not every line.
  await expect(page.getByTestId("speaker-announcer")).toContainText("Me: Okay");
  await page.keyboard.press("Control+M");
  await expect(page.getByText("1 marked")).toBeVisible();
  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByRole("heading", { name: "Meetings" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: "Live" })).toHaveCount(0);
});

test("switches language and theme", async ({ page }) => {
  await open(page, "/settings/general");
  const appLanguage = page.getByRole("group", { name: "App language" }).or(page.getByRole("radiogroup", { name: "App language" }));
  await page.getByRole("radio", { name: "Dark" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await appLanguage.getByRole("radio", { name: "Tiếng Việt" }).click();
  await expect(page.locator("html")).toHaveAttribute("lang", "vi");
  await expect(page.getByRole("navigation", { name: "Điều hướng chính" }).getByRole("link", { name: "Cuộc họp" })).toBeVisible();
  // Kept across reloads.
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect(page.getByRole("heading", { name: "Cài đặt" })).toBeVisible();
});

test("compact window: icons-only sidebar with accessible names", async ({ page }) => {
  await page.setViewportSize({ width: 960, height: 640 });
  await open(page);
  const nav = page.getByRole("navigation", { name: "Main" });
  await expect(nav.getByRole("link", { name: "People" })).toBeVisible();
  await expect(nav.getByText("People")).toHaveCount(0);
  const box = await nav.boundingBox();
  expect(box?.width).toBeLessThanOrEqual(56);
  // Nothing scrolls sideways at the minimum window size.
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
});
