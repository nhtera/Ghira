// SPDX-License-Identifier: Apache-2.0
// 15-I: Settings → Sync with computer on the scripted mock: pair, the paired
// card, unpair, unpair and wipe, the failed session, and the Desktop target.
import { expect, test } from "@playwright/test";
import { expectAccessible } from "./helpers";
import { mockHook, openSync } from "./sync-support";

const row = (page: import("@playwright/test").Page, name: RegExp | string) => page.getByRole("button", { name });

test("the row is not there without sync, and shows the pairing once there is", async ({ page }) => {
  await openSync(page, "/settings");
  await expect(row(page, /Sync with computer/)).toHaveCount(0);
  await openSync(page, "/settings", { sync: "off" });
  await expect(row(page, /Sync with computer/)).toContainText("Not paired");
  await openSync(page, "/settings", { sync: "paired" });
  await expect(row(page, /Sync with computer/)).toContainText("MacBook Pro");
});

test("not paired: scan, then the paired card with its last sync", async ({ page }) => {
  await openSync(page, "/settings/sync", { sync: "off" });
  await expect(page.getByRole("heading", { level: 1, name: "Sync with computer" })).toBeVisible();
  await expect(page.getByText("Local network only")).toBeVisible();
  await expect(page.getByText(/never goes through the internet/)).toBeVisible();
  // Hotspot guidance is there before anything fails.
  await expect(page.getByTestId("hotspot-help")).toContainText("Personal Hotspot");
  await expectAccessible(page);

  await page.getByRole("button", { name: "Scan the code" }).click();
  await expect(page.getByTestId("pair-viewfinder")).toHaveAttribute("data-phase", "scanning");
  await mockHook(page, "syncSimulateScan");
  await expect(page.getByText("MacBook Pro", { exact: true })).toBeVisible();
  await expect(page.getByText("Last synced 4 minutes ago")).toBeVisible();
  await expect(page.getByRole("button", { name: "Unpair", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Unpair and delete on MacBook Pro" })).toBeVisible();
  await expect(page.getByText("Local network only")).toBeVisible();
  await expectAccessible(page);
});

test("a failed scan can be retried", async ({ page }) => {
  await openSync(page, "/settings/sync", { sync: "off" });
  await mockHook(page, "syncFailNextScan", "expired");
  await page.getByRole("button", { name: "Scan the code" }).click();
  await expect(page.getByRole("status").filter({ hasText: "That code has expired" })).toBeVisible();
  await page.getByRole("button", { name: "Try again" }).click();
  await mockHook(page, "syncSimulateScan");
  await expect(page.getByText("Last synced 4 minutes ago")).toBeVisible();
});

test("Unpair asks first, keeps the meetings, and goes back to pairing", async ({ page }) => {
  await openSync(page, "/settings/sync", { sync: "paired" });
  await page.getByRole("button", { name: "Unpair", exact: true }).click();
  const sheet = page.getByRole("dialog", { name: "Unpair from MacBook Pro?" });
  await expect(sheet).toContainText("Meetings already on each one stay where they are.");
  await expectAccessible(page);
  await sheet.getByRole("button", { name: "Cancel" }).click();
  await expect(sheet).toHaveCount(0);
  await expect(page.getByText("MacBook Pro", { exact: true })).toBeVisible();

  await page.getByRole("button", { name: "Unpair", exact: true }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Unpair", exact: true }).click();
  await expect(page.getByRole("button", { name: "Scan the code" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Unpair", exact: true })).toHaveCount(0);
});

test("Unpair and delete says what happens, waits for the computer, then confirms", async ({ page }) => {
  await openSync(page, "/settings/sync", { sync: "paired" });
  await page.getByRole("button", { name: "Unpair and delete on MacBook Pro" }).click();
  const sheet = page.getByRole("dialog", { name: "Unpair and delete on MacBook Pro?" });
  await expect(sheet).toContainText("the next time they connect on the same Wi-Fi");
  await expect(sheet).toContainText("This can’t be undone on MacBook Pro.");
  await sheet.getByRole("button", { name: "Unpair and delete on MacBook Pro" }).click();
  await expect(page.getByTestId("wipe-pending")).toContainText("Waiting for MacBook Pro to delete the meetings it shares with this phone");
  await expect(page.getByRole("button", { name: /^Unpair/ })).toHaveCount(0);
  await expectAccessible(page);
  await mockHook(page, "syncSimulateWipeDone");
  await expect(page.getByText("MacBook Pro deleted them. They stay on this phone, which is now unpaired.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Scan the code" })).toBeVisible();
});

test("a failed session says why in words, and Sync now clears it", async ({ page }) => {
  await openSync(page, "/settings/sync", { sync: "error" });
  await expect(page.getByText("Couldn’t reach your computer. Check that both are on the same Wi-Fi.")).toBeVisible();
  await expect(page.getByText("2 items waiting to send")).toBeVisible();
  await page.getByRole("button", { name: "Sync now" }).click();
  await expect(page.getByText(/Couldn’t reach your computer/)).toHaveCount(0);
});

test("the computer unpaired this phone: it says so and offers pairing again", async ({ page }) => {
  await openSync(page, "/settings/sync", { sync: "paired" });
  await mockHook(page, "syncSimulateUnpairedByPeer");
  await expect(page.getByText("Unpaired by MacBook Pro. Pair again to keep syncing.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Scan the code" })).toBeVisible();
});

test("Settings home: My computer becomes the default target once paired", async ({ page }) => {
  await openSync(page, "/settings", { sync: "off" });
  const mine = page.getByRole("button", { name: /My computer/ });
  await expect(mine).toBeDisabled();
  await expect(mine).toContainText("Pair a computer first");
  await openSync(page, "/settings", { sync: "paired" });
  await expect(mine).toBeEnabled();
  await expect(mine).toContainText("MacBook Pro");
  await mine.click();
  await expect(mine.getByText("Selected")).toBeAttached();
});

test("delete everything asks about the paired computer, waits, then deletes here", async ({ page }) => {
  await openSync(page, "/settings/privacy", { sync: "paired" });
  await page.getByRole("button", { name: "Delete everything" }).click();
  const sheet = page.getByRole("dialog", { name: "Delete everything?" });
  const also = sheet.getByRole("switch", { name: "Also delete on MacBook Pro" });
  await expect(also).toBeChecked();
  await expect(sheet).toContainText("If MacBook Pro is reachable it deletes the meetings from this phone too.");
  await expectAccessible(page);
  await sheet.getByRole("textbox").fill("DELETE");
  await sheet.getByRole("button", { name: "Delete everything" }).click();
  await expect(page.getByRole("dialog", { name: "Waiting for MacBook Pro…" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Delete here only" })).toBeVisible();
  await expectAccessible(page);
  await mockHook(page, "syncSimulateWipeDone");
  await expect.poll(() => new URL(page.url()).hash, { timeout: 8000 }).toBe("#/onboarding");
});

test("delete everything here only: with the switch off", async ({ page }) => {
  await openSync(page, "/settings/privacy", { sync: "paired" });
  await page.getByRole("button", { name: "Delete everything" }).click();
  const sheet = page.getByRole("dialog", { name: "Delete everything?" });
  await sheet.getByRole("switch", { name: "Also delete on MacBook Pro" }).click();
  await sheet.getByRole("textbox").fill("DELETE");
  await sheet.getByRole("button", { name: "Delete everything" }).click();
  await expect.poll(() => new URL(page.url()).hash).toBe("#/onboarding");
});

test("delete everything: Delete here only skips the wait", async ({ page }) => {
  await openSync(page, "/settings/privacy", { sync: "paired" });
  await page.getByRole("button", { name: "Delete everything" }).click();
  const sheet = page.getByRole("dialog", { name: "Delete everything?" });
  await sheet.getByRole("textbox").fill("DELETE");
  await sheet.getByRole("button", { name: "Delete everything" }).click();
  await page.getByRole("button", { name: "Delete here only" }).click();
  await expect.poll(() => new URL(page.url()).hash).toBe("#/onboarding");
});

test("delete everything without a pairing has no extra question", async ({ page }) => {
  await openSync(page, "/settings/privacy");
  await page.getByRole("button", { name: "Delete everything" }).click();
  await expect(page.getByRole("switch", { name: /Also delete/ })).toHaveCount(0);
});

test("how long the computer may be away before the phone processes for itself is a choice", async ({ page }) => {
  await openSync(page, "/settings/sync", { sync: "paired" });
  await expect(page.getByText("If your computer is away")).toBeVisible();
  const twelve = page.getByRole("button", { name: /12 hours/ });
  await expect(twelve.getByText("Selected")).toBeAttached();
  await page.getByRole("button", { name: /1 day/ }).click();
  await expect(page.getByRole("button", { name: /1 day/ }).getByText("Selected")).toBeAttached();
  await expectAccessible(page);
});
