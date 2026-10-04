// SPDX-License-Identifier: Apache-2.0
// Settings → Privacy → Cloud request log: a read-only list of what was sent.
import { expect, test } from "@playwright/test";
import { expectAccessible, openApp } from "./helpers";

test("the privacy screen opens the log and back returns", async ({ page }) => {
  await openApp(page, "/settings/privacy");
  await page.getByRole("button", { name: /^Cloud request log/ }).click();
  await expect(page.getByRole("heading", { name: "Cloud request log", level: 1 })).toBeVisible();
  await page.getByRole("button", { name: /^Back/ }).click();
  await expect(page.getByRole("heading", { name: "Privacy and security", level: 1 })).toBeVisible();
});

test("says so when nothing was sent", async ({ page }) => {
  await openApp(page, "/settings/privacy/log");
  await expect(page.getByText("Nothing has been sent to the cloud.")).toBeVisible();
  await expectAccessible(page);
});

test("lists each request with meeting, provider, size and time, never content", async ({ page }) => {
  await openApp(page, "/settings/privacy");
  await page.evaluate(() => (window.__ghiSettingsMock!.cloudRequests = 3));
  await page.getByRole("button", { name: /^Cloud request log/ }).click();
  const items = page.locator("[data-screen=settings] li");
  await expect(items).toHaveCount(3);
  await expect(items.first()).toContainText("Weekly sync");
  await expect(items.first()).toContainText("Anthropic · claude-sonnet-5-5");
  await expect(items.first()).toContainText("900 in · 300 out tokens");
  await expect(items.nth(1)).toContainText("Untitled meeting");
  await expectAccessible(page);
});

test("Vietnamese at 200% text stays accessible", async ({ page }) => {
  await openApp(page, "/settings/privacy", { lang: "vi", scale: 2 });
  await page.evaluate(() => (window.__ghiSettingsMock!.cloudRequests = 2));
  await page.getByRole("button", { name: /^Nhật ký yêu cầu đám mây/ }).click();
  await expect(page.getByText("Cuộc họp chưa đặt tên")).toBeVisible();
  await expectAccessible(page);
});
