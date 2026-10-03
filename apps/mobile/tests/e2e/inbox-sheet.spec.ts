// SPDX-License-Identifier: Apache-2.0
// M5 inside the app (16-J): files the share extension left behind wait for a
// language and target; importing shows a toast; nothing shows while locked.
import { expect, test, type Page } from "@playwright/test";
import { expectAccessible, openApp } from "./helpers";

const item = (id: string, over: Record<string, unknown> = {}) => ({ id, name: `${id}.m4a`, sizeBytes: 12_000_000, language: "auto", target: "phone", state: "pending", reason: null, ...over });
const seed = (page: Page, items: unknown[]) => page.evaluate((i) => window.__ghiSettingsMock!.setInbox(i as never), items);

test.beforeEach(async ({ page }) => {
  await openApp(page, "/settings");
});

test("waiting files raise a banner; Review opens the choices", async ({ page }) => {
  await seed(page, [item("standup"), item("review")]);
  const banner = page.getByRole("status").filter({ hasText: "2 files are waiting to import" });
  await expect(banner).toBeVisible();
  await banner.getByRole("button", { name: "Review" }).click();
  const dialog = page.getByRole("dialog", { name: "Waiting to import" });
  await expect(dialog.getByText("standup.m4a")).toBeVisible();
  await expect(dialog.getByRole("button", { name: "My computer" }).first()).toBeDisabled();
  await expectAccessible(page);
});

test("confirming imports the file, shows the toast, and nothing else was imported", async ({ page }) => {
  await seed(page, [item("standup"), item("review")]);
  await page.getByRole("button", { name: "Review" }).click();
  const dialog = page.getByRole("dialog", { name: "Waiting to import" });
  const first = dialog.getByRole("listitem").filter({ hasText: "standup.m4a" });
  await first.getByRole("button", { name: "VI" }).click();
  await first.getByRole("button", { name: "Import" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Added to Ghira" })).toBeVisible();
  await expect(dialog.getByText("standup.m4a")).toHaveCount(0);
  expect(await page.evaluate(() => window.__ghiSettingsMock!.args.inboxConfirm?.slice(1))).toEqual(["vi", "phone"]);
  expect(await page.evaluate(() => window.__ghiSettingsMock!.calls.inboxConfirm)).toBe(1);
  await expect(dialog.getByText("review.m4a")).toBeVisible();
});

test("the last file imported closes the sheet and the toast shows on its own", async ({ page }) => {
  await seed(page, [item("only")]);
  await page.getByRole("button", { name: "Review" }).click();
  const dialog = page.getByRole("dialog", { name: "Waiting to import" });
  await dialog.getByRole("button", { name: "Import" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("status").filter({ hasText: "Added to Ghira" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Review" })).toHaveCount(0);
});

test("a rejected file explains why and can be removed", async ({ page }) => {
  // The banner counts files that wait for a choice, so a pending one opens the sheet.
  await seed(page, [item("voice-memo", { name: "voice-memo.caf", state: "rejected", reason: "unsupportedType" }), item("b")]);
  await page.getByRole("button", { name: "Review" }).click();
  const dialog = page.getByRole("dialog", { name: "Waiting to import" });
  await expect(dialog.getByRole("alert")).toContainText("This file type isn’t supported.");
  await dialog.getByRole("button", { name: "Remove voice-memo.caf" }).click();
  await expect(dialog.getByText("voice-memo.caf")).toHaveCount(0);
});

test("a busy core keeps the file and says why", async ({ page }) => {
  await seed(page, [item("a")]);
  await page.evaluate(() => (window.__ghiSettingsMock!.busy = true));
  await page.getByRole("button", { name: "Review" }).click();
  const dialog = page.getByRole("dialog", { name: "Waiting to import" });
  await dialog.getByRole("button", { name: "Import" }).click();
  await expect(dialog.getByRole("alert")).toContainText("A recording or import is running");
  await expect(dialog.getByText("a.m4a")).toBeVisible();
});

test("while the app is locked the inbox stays hidden, then shows after unlock", async ({ page }) => {
  await page.evaluate(() => {
    window.__ghiSettingsMock!.faceIdOk = false;
    window.__ghiSettingsMock!.locked = true;
    window.dispatchEvent(new Event("focus"));
  });
  await seed(page, [item("a")]);
  await expect(page.getByRole("button", { name: "Review" })).toHaveCount(0);
  await page.evaluate(() => (window.__ghiSettingsMock!.faceIdOk = true));
  await page.getByRole("button", { name: "Unlock with Face ID" }).click();
  await expect(page.getByRole("button", { name: "Review" })).toBeVisible();
});

test("locking clears the inbox from the screen, unlocking brings it back", async ({ page }) => {
  await seed(page, [item("a")]);
  await expect(page.getByRole("button", { name: "Review" })).toBeVisible();
  await page.getByRole("button", { name: "Review" }).click();
  await page.evaluate(() => {
    window.__ghiSettingsMock!.faceIdOk = false;
    window.__ghiSettingsMock!.locked = true;
    window.dispatchEvent(new Event("focus"));
  });
  await expect(page.getByRole("dialog", { name: "Waiting to import" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Review" })).toHaveCount(0);
  await page.evaluate(() => (window.__ghiSettingsMock!.faceIdOk = true));
  await page.getByRole("button", { name: "Unlock with Face ID" }).click();
  await expect(page.getByRole("button", { name: "Review" })).toBeVisible();
});
