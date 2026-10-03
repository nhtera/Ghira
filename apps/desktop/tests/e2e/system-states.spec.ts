// SPDX-License-Identifier: Apache-2.0
// D12 app-level system states on the mocked core. Copy not merged yet shows as
// its key, so banners are found by their `data-banner` and by existing copy.
import { expect, test, type Page } from "@playwright/test";

const open = (page: Page, query = "") => page.goto(`/?platform=win${query}#/meetings`);
const core = (page: Page, event: object) => page.evaluate((e) => (window as unknown as { __ghiMock: { simulateCoreEvent: (e: object) => void } }).__ghiMock.simulateCoreEvent(e), event);

test("a crash-recovered meeting shows a dialog that opens it", async ({ page }) => {
  await open(page, "&recovered=1");
  const dialog = page.getByRole("dialog", { name: /closed unexpectedly/ });
  await expect(dialog).toContainText("saved up to 25:20");
  await dialog.getByRole("button", { name: "Recover and write notes" }).click();
  await expect(page).toHaveURL(/#\/meetings\/sample-0\/notes/);
  // Told once per launch: it doesn't come back on the next screen.
  await expect(dialog).toHaveCount(0);
});

test("discarding a recovered meeting asks first", async ({ page }) => {
  await open(page, "&recovered=1");
  const dialog = page.getByRole("dialog", { name: /closed unexpectedly/ });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Discard recording" }).click();
  // The same dialog asks before deleting; Cancel goes back.
  await expect(dialog.getByRole("alert")).toContainText("This can’t be undone");
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog.getByRole("alert")).toHaveCount(0);
  await dialog.getByRole("button", { name: "Discard recording" }).click();
  await dialog.getByRole("button", { name: "Discard recording" }).click();
  await expect(dialog).toHaveCount(0);
});

test("no banner without a crash", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("heading", { name: "Meetings" })).toBeVisible();
  await expect(page.locator("[data-banner]")).toHaveCount(0);
});

test("a capture error from the core shows once and can be dismissed", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("heading", { name: "Meetings" })).toBeVisible();
  const event = { type: "error", meeting: null, kind: "capture", message: "device in exclusive mode" };
  await core(page, event);
  await core(page, event);
  const banner = page.locator("[data-banner=capture]");
  await expect(banner).toHaveCount(1);
  await expect(banner).toContainText("Another app is using the microphone");
  await banner.locator("button").click();
  await expect(banner).toHaveCount(0);
});

test("a storage error blocks the window with a retry", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("heading", { name: "Meetings" })).toBeVisible();
  await core(page, { type: "error", meeting: null, kind: "storage", message: "disk I/O error" });
  const alert = page.locator("[data-banner=storage]");
  await expect(alert).toContainText("disk I/O error");
  await expect(alert.getByRole("button", { name: "Try again" })).toBeFocused();
});

test("a storage write failure during a recording is a banner, not a block", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("heading", { name: "Meetings" })).toBeVisible();
  await core(page, { type: "error", meeting: "m1", kind: "storage", message: "write failed" });
  await expect(page.locator("[data-banner=storage-write]")).toContainText("write failed");
  await expect(page.locator("[data-banner=storage]")).toHaveCount(0);
});
