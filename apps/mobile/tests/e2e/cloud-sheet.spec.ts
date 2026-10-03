// SPDX-License-Identifier: Apache-2.0
// The cloud send sheet (16-J): shows exactly what would be sent, never sends
// before the click, is blocked for a meeting with cloud AI off, and a failed
// send leaves the notes on the phone.
import { expect, test, type Page } from "@playwright/test";
import { expectAccessible, openApp } from "./helpers";

const cloudSends = (page: Page) => page.evaluate(() => window.__ghiSettingsMock!.calls.cloudSend ?? 0);
const openSheet = (page: Page, meetingId = "m-1") => page.evaluate((id) => window.dispatchEvent(new CustomEvent("ghi:open-cloud-sheet", { detail: { meetingId: id } })), meetingId);
const sheet = (page: Page) => page.getByRole("dialog", { name: "Improve with cloud" });

test.beforeEach(async ({ page }) => {
  await openApp(page, "/settings");
  await page.evaluate(() => {
    window.__ghiSettingsMock!.keys.anthropic = true;
    window.__ghiSettingsMock!.offerCloud(true);
  });
});

test("until cloud notes are offered the sheet points to the setting and calls nothing", async ({ page }) => {
  await page.evaluate(() => window.__ghiSettingsMock!.offerCloud(false));
  await openSheet(page);
  const dialog = sheet(page);
  await expect(dialog.getByText(/Cloud notes are off\./)).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Send" })).toBeDisabled();
  expect(await page.evaluate(() => window.__ghiSettingsMock!.calls.cloudPreview ?? 0)).toBe(0);
  await dialog.getByRole("button", { name: "Open Cloud notes" }).click();
  await expect(page.getByRole("heading", { name: "Cloud notes", level: 1 })).toBeVisible();
  await page.getByRole("switch", { name: "Offer cloud notes" }).click();
  await openSheet(page);
  await expect(sheet(page).locator("pre")).toBeVisible();
});

test("the core refusing with cloudOff shows the same pointer", async ({ page }) => {
  // The setting looked on when the sheet loaded, then the core says it is off.
  await page.evaluate(() => window.__ghiSettingsMock!.offerCloud(true));
  await openSheet(page);
  await expect(sheet(page).locator("pre")).toBeVisible();
  await page.evaluate(() => window.__ghiSettingsMock!.offerCloud(false));
  await sheet(page).getByRole("button", { name: "Send" }).click();
  await expect(sheet(page).getByText(/Cloud notes are off\./)).toBeVisible();
  expect(await cloudSends(page)).toBe(1);
});

test("the meeting view offers cloud notes only after the setting is on", async ({ page }) => {
  await page.evaluate(() => window.__ghiSettingsMock!.offerCloud(false));
  await page.evaluate(() => (location.hash = "#/meetings/m-notes"));
  await expect(page.getByRole("heading", { level: 1, name: "Product sync tuần 39" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Improve with cloud" })).toHaveCount(0);
  await page.evaluate(() => window.__ghiSettingsMock!.offerCloud(true));
  await page.evaluate(() => (location.hash = "#/meetings"));
  await page.evaluate(() => (location.hash = "#/meetings/m-notes"));
  await expect(page.getByRole("button", { name: "Improve with cloud" })).toBeVisible();
});

test("previews the exact text and sends nothing until the click", async ({ page }) => {
  await openSheet(page);
  const dialog = sheet(page);
  await expect(dialog.getByRole("heading", { name: "Exactly what will be sent" })).toBeVisible();
  await expect(dialog.getByText("About 1,240 tokens")).toBeVisible();
  await expect(dialog.getByText("Sent to api.anthropic.com")).toBeVisible();
  await expect(dialog.locator("pre")).toContainText("<<PERSON_1>>");
  await expect(dialog.locator("pre")).not.toContainText("Nguyễn Văn An");
  await expectAccessible(page);

  // Redaction off: the exact bytes change, and a warning appears. Still nothing sent.
  const hide = dialog.getByRole("switch", { name: "Hide names and personal data" });
  await hide.click();
  await expect(dialog.locator("pre")).toContainText("Nguyễn Văn An");
  await expect(dialog.getByText("Names and personal data will be sent as written.")).toBeVisible();
  await hide.click();
  await expect(dialog.locator("pre")).toContainText("<<PERSON_1>>");
  expect(await cloudSends(page)).toBe(0);

  await dialog.getByRole("button", { name: "Send" }).click();
  await expect(dialog.getByText("Notes were rewritten with the cloud.")).toBeVisible();
  expect(await cloudSends(page)).toBe(1);
  await dialog.getByRole("button", { name: "Close" }).first().click();
  await expect(dialog).toHaveCount(0);
});

test("shows the warnings, the retention note and the hidden kinds in words", async ({ page }) => {
  await page.evaluate(() => {
    window.__ghiSettingsMock!.warnings = ["A phone number may remain on line 4."];
    window.__ghiSettingsMock!.retentionNote = "kept for 30 days";
  });
  await openSheet(page);
  const dialog = sheet(page);
  await expect(dialog.getByText("This still looks like personal data:")).toBeVisible();
  await expect(dialog.getByText("A phone number may remain on line 4.")).toBeVisible();
  await expect(dialog.getByText("Provider retention: kept for 30 days")).toBeVisible();
  await expect(dialog.getByText("Hidden: People ×2")).toBeVisible();
});

test("cancel closes without sending", async ({ page }) => {
  await openSheet(page);
  const dialog = sheet(page);
  await expect(dialog.locator("pre")).toBeVisible();
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toHaveCount(0);
  expect(await cloudSends(page)).toBe(0);
});

test("a meeting with cloud AI off shows a disabled Send", async ({ page }) => {
  await page.evaluate(() => window.__ghiSettingsMock!.cloudLocked.push("locked-1"));
  await openSheet(page, "locked-1");
  const dialog = sheet(page);
  await expect(dialog.getByText(/Cloud is off for this meeting/)).toBeVisible();
  await expect(dialog.getByText(/Never send to cloud/)).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Send" })).toBeDisabled();
  await expect(dialog.locator("pre")).toHaveCount(0);
  await expectAccessible(page);
  expect(await cloudSends(page)).toBe(0);
});

test("a meeting known to be locked skips the preview call", async ({ page }) => {
  await page.evaluate(() => window.dispatchEvent(new CustomEvent("ghi:open-cloud-sheet", { detail: { meetingId: "m-1", cloudLocked: true } })));
  await expect(sheet(page).getByRole("button", { name: "Send" })).toBeDisabled();
  expect(await page.evaluate(() => window.__ghiSettingsMock!.calls.cloudPreview ?? 0)).toBe(0);
});

test("a failed send says the notes stay on the phone", async ({ page }) => {
  await page.evaluate(() => window.__ghiSettingsMock!.failSend.push("m-1"));
  await openSheet(page);
  const dialog = sheet(page);
  await dialog.getByRole("button", { name: "Send" }).click();
  await expect(dialog.getByRole("alert")).toContainText("your notes stay on this phone");
  await expect(dialog.getByText(/may already have reached/)).toHaveCount(0);
  expect(await cloudSends(page)).toBe(1);
});

test("without a key it offers to add one", async ({ page }) => {
  await page.evaluate(() => (window.__ghiSettingsMock!.keys.anthropic = false));
  await openSheet(page);
  const dialog = sheet(page);
  await expect(dialog.getByText("Add a provider key first.")).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Send" })).toBeDisabled();
  await dialog.getByRole("button", { name: "Add a key" }).click();
  await expect(page.getByRole("heading", { name: "Cloud notes", level: 1 })).toBeVisible();
  expect(await cloudSends(page)).toBe(0);
});
