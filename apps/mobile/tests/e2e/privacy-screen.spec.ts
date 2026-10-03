// SPDX-License-Identifier: Apache-2.0
// Settings -> Privacy and security (16-J): network line, app lock, retention,
// export everything (password) and delete everything (typed confirmation).
import { expect, test, type Page } from "@playwright/test";
import { expectAccessible, openApp } from "./helpers";

const hook = <T>(page: Page, fn: () => T) => page.evaluate(fn);
const row = (page: Page, name: string | RegExp) => page.getByRole("button", { name });

test("the network line says local only, with the requests made today", async ({ page }) => {
  await openApp(page, "/settings/privacy");
  await expect(page.getByText("Local only · 0 requests today")).toBeVisible();
  await hook(page, () => (window.__ghiSettingsMock!.cloudRequests = 1));
  await page.evaluate(() => (location.hash = "#/settings"));
  await row(page, /^Privacy and security/).click();
  await expect(page.getByText("Local only · 1 request today")).toBeVisible();
  await expectAccessible(page);
});

test.describe("app lock", () => {
  test("a phone without a passcode explains why the lock stays off", async ({ page }) => {
    await openApp(page, "/settings/privacy");
    await page.evaluate(() => (window.__ghiSettingsMock!.noAuthMethod = true));
    await page.getByRole("switch", { name: "Require Face ID" }).click();
    await expect(page.getByRole("alert")).toContainText("Turn on a passcode in Settings");
    await expect(page.getByRole("switch", { name: "Require Face ID" })).not.toBeChecked();
  });

  test("turning it on and off asks Face ID each time", async ({ page }) => {
    await openApp(page, "/settings/privacy");
    const lock = page.getByRole("switch", { name: "Require Face ID" });
    await expect(lock).not.toBeChecked();
    await expect(page.getByText("Lock after")).toHaveCount(0);
    await lock.click();
    await expect(lock).toBeChecked();
    expect(await hook(page, () => window.__ghiSettingsMock!.faceIdPrompts)).toBe(1);
    // The idle time only changes the setting; it does not ask again.
    await row(page, /^5 minutes/).click();
    await expect(row(page, /^5 minutes/).getByText("Selected")).toBeVisible();
    expect(await hook(page, () => window.__ghiSettingsMock!.faceIdPrompts)).toBe(1);
    await lock.click();
    await expect(lock).not.toBeChecked();
    expect(await hook(page, () => window.__ghiSettingsMock!.faceIdPrompts)).toBe(2);
  });

  test("it stays off when Face ID does not match", async ({ page }) => {
    await openApp(page, "/settings/privacy");
    await hook(page, () => (window.__ghiSettingsMock!.faceIdOk = false));
    await page.getByRole("switch", { name: "Require Face ID" }).click();
    await expect(page.getByRole("alert")).toContainText("Couldn’t save that change.");
    await expect(page.getByRole("switch", { name: "Require Face ID" })).not.toBeChecked();
  });
});

test("audio retention round-trips", async ({ page }) => {
  await openApp(page, "/settings/privacy");
  await expect(row(page, /^Until I delete the meeting/).getByText("Selected")).toBeVisible();
  await row(page, /^90 days/).click();
  await expect(row(page, /^90 days/).getByText("Selected")).toBeVisible();
  await page.evaluate(() => (location.hash = "#/settings"));
  await row(page, /^Privacy and security/).click();
  await expect(row(page, /^90 days/).getByText("Selected")).toBeVisible();
  await expect(page.getByText("Transcripts and notes stay.")).toBeVisible();
});

test.describe("export everything", () => {
  test("needs a password of at least 8 characters", async ({ page }) => {
    await openApp(page, "/settings/privacy");
    await row(page, /^Export everything/).click();
    const dialog = page.getByRole("dialog", { name: "Export everything" });
    const share = dialog.getByRole("button", { name: "Choose where to save" });
    const pw = dialog.getByLabel("Password");
    await expect(pw).toHaveAttribute("type", "password");
    await expect(pw).toHaveAttribute("autocomplete", "off");
    await expect(share).toBeDisabled();
    await pw.fill("short");
    await expect(share).toBeDisabled();
    await pw.fill("correct horse");
    await expect(share).toBeEnabled();
    await expectAccessible(page);
    expect(await hook(page, () => window.__ghiSettingsMock!.calls.privacyExportAllShare ?? 0)).toBe(0);
    await share.click();
    await expect(dialog.getByText("Ready to share.")).toBeVisible();
    expect(await hook(page, () => window.__ghiSettingsMock!.exportedWith)).toBe("correct horse");
  });
});

test.describe("delete everything", () => {
  test("requires the typed phrase, then returns to first launch", async ({ page }) => {
    await openApp(page, "/settings/privacy");
    await page.evaluate(() => localStorage.setItem("ghi.search.recent", JSON.stringify(["ngân sách"])));
    await row(page, /^Delete everything/).click();
    const dialog = page.getByRole("dialog", { name: "Delete everything?" });
    const confirm = dialog.getByRole("button", { name: "Delete everything" });
    await expect(confirm).toBeDisabled();
    const field = dialog.getByLabel("Type DELETE to confirm");
    await field.fill("del");
    await expect(confirm).toBeDisabled();
    await expectAccessible(page);
    expect(await hook(page, () => window.__ghiSettingsMock!.calls.privacyDeleteAll ?? 0)).toBe(0);
    await field.fill("delete");
    await expect(confirm).toBeEnabled();
    await confirm.click();
    await expect.poll(() => page.evaluate(() => location.hash)).toBe("#/onboarding");
    // Recent searches are words from meetings: they are gone too.
    expect(await page.evaluate(() => localStorage.getItem("ghi.search.recent"))).toBeNull();
    expect(await hook(page, () => window.__ghiSettingsMock!.wiped)).toBe(true);
  });

  test("is refused while a recording or import runs", async ({ page }) => {
    await openApp(page, "/settings/privacy");
    await hook(page, () => (window.__ghiSettingsMock!.busy = true));
    await row(page, /^Delete everything/).click();
    const dialog = page.getByRole("dialog", { name: "Delete everything?" });
    await dialog.getByLabel("Type DELETE to confirm").fill("DELETE");
    await dialog.getByRole("button", { name: "Delete everything" }).click();
    await expect(dialog.getByRole("alert")).toContainText("A recording or import is running");
    expect(await hook(page, () => window.__ghiSettingsMock!.wiped)).toBe(false);
    await expect.poll(() => page.evaluate(() => location.hash)).toBe("#/settings/privacy");
  });

  test("in Vietnamese the phrase is XOÁ and accents are optional", async ({ page }) => {
    await openApp(page, "/settings/privacy", { lang: "vi" });
    await page.getByRole("button", { name: /^Xoá toàn bộ/ }).click();
    const dialog = page.getByRole("dialog", { name: "Xoá toàn bộ dữ liệu?" });
    const confirm = dialog.getByRole("button", { name: "Xoá toàn bộ" });
    await expect(dialog.getByLabel("Gõ XOÁ để xác nhận")).toBeVisible();
    await dialog.getByLabel("Gõ XOÁ để xác nhận").fill("xoa");
    await expect(confirm).toBeEnabled();
  });
});
