// SPDX-License-Identifier: Apache-2.0
// Calendar (P1): Settings → Calendar connects the phone's calendar (iOS asks
// the first time), and the record screen then names the event in progress.
// On the scripted mock (`window.__ghiCalendar`); the matrix checks axe and
// the baselines in both languages and at 200% text.
import { expect, test, type Page } from "@playwright/test";
import { expectAccessible, openApp } from "./helpers";
import { openRecord } from "./record-support";
import { visualMatrix } from "./settings-matrix";

const cal = (page: Page, fields: object) => page.evaluate((f) => Object.assign(window.__ghiCalendar as object, f), fields);
const calls = (page: Page, name: string) => page.evaluate((n) => window.__ghiCalendar!.calls[n] ?? 0, name);

test("it is off and unasked until the user connects it", async ({ page }) => {
  await openApp(page, "/settings");
  await expect(page.getByRole("button", { name: /^Calendar\s*Off/ })).toBeVisible();
  await page.getByRole("button", { name: /^Calendar/ }).click();
  await expect(page.getByRole("heading", { name: "Calendar", level: 1 })).toBeVisible();
  await expect(page.getByText("Calendar is off.")).toBeVisible();
  expect(await calls(page, "calendarConnect")).toBe(0);
  await expectAccessible(page);

  await page.getByRole("button", { name: "Connect calendar" }).click();
  await expect(page.getByText("Reading your calendar on this phone.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Turn off calendar" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Connect calendar" })).toHaveCount(0);
  await expectAccessible(page);

  // The row on the settings home says it too.
  await page.getByRole("button", { name: /^Back/ }).click();
  await expect(page.getByRole("button", { name: /^Calendar\s*Connected/ })).toBeVisible();
});

test("a refused prompt points to Settings and nothing is connected", async ({ page }) => {
  await openApp(page, "/settings");
  await cal(page, { promptAnswer: "denied" });
  await page.getByRole("button", { name: /^Calendar/ }).click();
  await page.getByRole("button", { name: "Connect calendar" }).click();
  await expect(page.getByText(/can’t read your calendar/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Open Settings" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Connect calendar" })).toHaveCount(0);
  await expectAccessible(page);
  await page.getByRole("button", { name: /^Back/ }).click();
  await expect(page.getByRole("button", { name: /^Calendar\s*Off/ })).toBeVisible();
});

test("the record screen names the event in progress, only while connected", async ({ page }) => {
  await openRecord(page);
  await expect(page.getByTestId("calendar-card")).toHaveCount(0);

  await cal(page, { access: "authorized", connected: true });
  // Away and back: the screen looks again when it opens.
  await page.evaluate(() => (location.hash = "#/meetings"));
  await page.evaluate(() => (location.hash = "#/record"));
  const card = page.getByTestId("calendar-card");
  await expect(card).toContainText("Weekly sync");
  await expect(card).toContainText("This recording will be named after it.");
  await expectAccessible(page);

  await cal(page, { connected: false });
  await page.evaluate(() => (location.hash = "#/meetings"));
  await page.evaluate(() => (location.hash = "#/record"));
  await expect(page.getByTestId("calendar-card")).toHaveCount(0);
});

test("turning it off says what stays with iOS", async ({ page }) => {
  await openApp(page, "/settings");
  await cal(page, { access: "authorized", connected: true });
  await page.getByRole("button", { name: /^Calendar/ }).click();
  await page.getByRole("button", { name: "Turn off calendar" }).click();
  await expect(page.getByText(/Calendar turned off\. To take back the permission too/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Connect calendar" })).toBeVisible();
  expect(await calls(page, "calendarDisconnect")).toBe(1);
});

test("the record note is accessible in Vietnamese at 200% text", async ({ page }) => {
  await openRecord(page, { lang: "vi", scale: 2 });
  await cal(page, { access: "authorized", connected: true });
  await page.evaluate(() => (location.hash = "#/meetings"));
  await page.evaluate(() => (location.hash = "#/record"));
  await expect(page.getByTestId("calendar-card")).toContainText("Weekly sync");
  await expect(page.getByTestId("calendar-card")).toContainText("Từ lịch của bạn");
  await expectAccessible(page);
});

const state = (fields: object) => (page: Page) => cal(page, fields);
const settle = async (page: Page) => {
  await expect(page.getByRole("heading", { name: /^(Calendar|Lịch)$/, level: 1 })).toBeVisible();
  await expect(page.getByRole("button", { name: /Connect calendar|Kết nối lịch|Turn off calendar|Tắt lịch|Open Settings|Mở Cài đặt/ })).toBeVisible();
};

visualMatrix("calendar", [
  { name: "off", route: "/settings/calendar", setup: state({}), ready: settle },
  { name: "denied", route: "/settings/calendar", setup: state({ access: "denied" }), ready: settle },
  { name: "connected", route: "/settings/calendar", setup: state({ access: "authorized", connected: true }), ready: settle },
]);
