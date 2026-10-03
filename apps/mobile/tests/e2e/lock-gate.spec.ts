// SPDX-License-Identifier: Apache-2.0
// The app-lock gate (16-J): covers everything while locked, unlocks with Face
// ID, and stays locked when it does not match.
import { expect, test, type Page } from "@playwright/test";
import { expectAccessible, openApp } from "./helpers";

const lockApp = (page: Page) =>
  page.evaluate(() => {
    window.__ghiSettingsMock!.locked = true;
    window.dispatchEvent(new Event("focus"));
  });
const gate = (page: Page) => page.getByRole("dialog", { name: "Ghira is locked" });

test("nothing shows through the gate, and Face ID unlocks", async ({ page }) => {
  await openApp(page, "/settings");
  await expect(page.getByRole("heading", { name: "Settings", level: 1 })).toBeVisible();
  await expect(gate(page)).toHaveCount(0);

  // Face ID does not match the first time (the automatic prompt).
  await page.evaluate(() => (window.__ghiSettingsMock!.faceIdOk = false));
  await lockApp(page);
  await expect(gate(page)).toBeVisible();
  await expect(gate(page).getByRole("alert")).toContainText("That didn’t work");
  expect(await page.evaluate(() => window.__ghiSettingsMock!.faceIdPrompts)).toBe(1);
  // The page behind is inert: focus and assistive tech cannot reach it.
  await expect(page.locator("#root")).toHaveJSProperty("inert", true);
  await expectAccessible(page);

  // Then it matches.
  await page.evaluate(() => (window.__ghiSettingsMock!.faceIdOk = true));
  await gate(page).getByRole("button", { name: "Unlock with Face ID" }).click();
  await expect(gate(page)).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Settings", level: 1 })).toBeVisible();
  await expect(page.locator("#root")).toHaveJSProperty("inert", false);
  expect(await page.evaluate(() => window.__ghiSettingsMock!.faceIdPrompts)).toBe(2);
});

test("the gate asks for Face ID by itself and unlocks on a match", async ({ page }) => {
  await openApp(page, "/settings");
  await lockApp(page);
  await expect(gate(page)).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Settings", level: 1 })).toBeVisible();
  expect(await page.evaluate(() => window.__ghiSettingsMock!.faceIdPrompts)).toBe(1);
});

test("locking again after an unlock shows the gate again", async ({ page }) => {
  await openApp(page, "/settings");
  await page.evaluate(() => (window.__ghiSettingsMock!.faceIdOk = false));
  await lockApp(page);
  await expect(gate(page)).toBeVisible();
  await page.evaluate(() => (window.__ghiSettingsMock!.faceIdOk = true));
  await gate(page).getByRole("button", { name: "Unlock with Face ID" }).click();
  await expect(gate(page)).toHaveCount(0);
  await page.evaluate(() => (window.__ghiSettingsMock!.faceIdOk = false));
  await lockApp(page);
  await expect(gate(page)).toBeVisible();
});

test.describe("the gate is the top layer", () => {
  test("an open cloud sheet is closed by the lock, and Unlock stays clickable", async ({ page }) => {
    await openApp(page, "/settings");
    await page.evaluate(() => {
      window.__ghiSettingsMock!.keys.anthropic = true;
      window.__ghiSettingsMock!.faceIdOk = false;
      window.dispatchEvent(new CustomEvent("ghi:open-cloud-sheet", { detail: { meetingId: "m-1" } }));
    });
    await expect(page.getByRole("dialog", { name: "Improve with cloud" })).toBeVisible();
    await lockApp(page);
    await expect(gate(page)).toBeVisible();
    await expect(page.getByRole("dialog", { name: "Improve with cloud" })).toHaveCount(0);
    // Every other child of <body> is inert; the gate is not.
    expect(await page.evaluate(() => [...document.body.children].filter((e) => !e.hasAttribute("data-lock-gate") && !(e as HTMLElement).inert && e.tagName !== "SCRIPT").length)).toBe(0);
    await page.evaluate(() => (window.__ghiSettingsMock!.faceIdOk = true));
    await gate(page).getByRole("button", { name: "Unlock with Face ID" }).click();
    await expect(gate(page)).toHaveCount(0);
    // Back on a fresh screen: the sheet does not come back.
    await expect(page.getByRole("heading", { name: "Settings", level: 1 })).toBeVisible();
    await expect(page.getByRole("dialog")).toHaveCount(0);
  });

  test("a sheet inside a screen is gone too, and the gate takes the clicks", async ({ page }) => {
    await openApp(page, "/settings/privacy");
    await page.getByRole("button", { name: /^Export everything/ }).click();
    await expect(page.getByRole("dialog", { name: "Export everything" })).toBeVisible();
    await page.evaluate(() => (window.__ghiSettingsMock!.faceIdOk = false));
    await lockApp(page);
    await expect(gate(page)).toBeVisible();
    await expect(page.getByRole("dialog", { name: "Export everything" })).toHaveCount(0);
    await page.evaluate(() => (window.__ghiSettingsMock!.faceIdOk = true));
    await gate(page).getByRole("button", { name: "Unlock with Face ID" }).click();
    await expect(page.getByRole("heading", { name: "Privacy and security", level: 1 })).toBeVisible();
    await expect(page.getByRole("dialog")).toHaveCount(0);
  });
});

test.describe("launch", () => {
  test("shows nothing until the core has started, then the screen", async ({ page }) => {
    await page.goto("/?starting=1200#/settings");
    await page.waitForFunction(() => Boolean(window.__ghiMock));
    await expect(page.getByTestId("app-lock-gate")).toBeVisible();
    await expect(page.getByRole("heading", { name: "Settings", level: 1 })).toHaveCount(0);
    await expect(page.getByRole("heading", { name: "Settings", level: 1 })).toBeVisible({ timeout: 8000 });
    await expect(page.getByTestId("app-lock-gate")).toHaveCount(0);
  });

  test("a locked launch shows only the gate until Face ID matches", async ({ page }) => {
    await page.goto("/?locked=1#/settings");
    await page.waitForFunction(() => Boolean(window.__ghiMock));
    // The automatic prompt matched (faceIdOk is true by default), so unlock follows.
    await expect(page.getByRole("heading", { name: "Settings", level: 1 })).toBeVisible();
    expect(await page.evaluate(() => window.__ghiSettingsMock!.faceIdPrompts)).toBe(1);
  });
});

test.describe("hidden page", () => {
  const hide = (page: Page, hidden: boolean) =>
    page.evaluate((h) => {
      Object.defineProperty(document, "visibilityState", { value: h ? "hidden" : "visible", configurable: true });
      document.dispatchEvent(new Event("visibilitychange"));
    }, hidden);

  test("with the lock on the screen is covered as soon as the page is hidden", async ({ page }) => {
    await openApp(page, "/settings/privacy");
    await page.getByRole("switch", { name: "Require Face ID" }).click();
    await expect(page.getByRole("switch", { name: "Require Face ID" })).toBeChecked();
    await hide(page, true);
    await expect(page.getByTestId("app-lock-gate")).toBeVisible();
    await hide(page, false);
    await expect(page.getByTestId("app-lock-gate")).toHaveCount(0);
    await expect(page.getByRole("heading", { name: "Privacy and security", level: 1 })).toBeVisible();
  });

  test("without the lock nothing is covered", async ({ page }) => {
    await openApp(page, "/settings");
    await hide(page, true);
    await expect(page.getByTestId("app-lock-gate")).toHaveCount(0);
  });
});

test("no passcode on the phone: the gate says so", async ({ page }) => {
  await openApp(page, "/settings");
  await page.evaluate(() => (window.__ghiSettingsMock!.noAuthMethod = true));
  await lockApp(page);
  await expect(gate(page).getByRole("alert")).toContainText("Turn on a passcode in Settings");
});

test("the core's lock-changed event locks and unlocks without a poll", async ({ page }) => {
  await openApp(page, "/settings");
  await page.evaluate(() => {
    window.__ghiSettingsMock!.faceIdOk = false;
    window.__ghiSettingsMock!.setLocked(true);
  });
  await expect(gate(page)).toBeVisible();
  await page.evaluate(() => window.__ghiSettingsMock!.setLocked(false));
  await expect(gate(page)).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Settings", level: 1 })).toBeVisible();
});
