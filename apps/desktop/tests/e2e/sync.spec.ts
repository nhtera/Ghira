// SPDX-License-Identifier: Apache-2.0
// Phase 15-G on the mocked core: Settings > Sync (pair sheet with expiry and
// a new code, devices, unpair and wipe, hotspot guidance), the conflict
// banner, the mass-delete question and the delete messages. Counterpart:
// sync-visual.spec.ts (axe and baselines).
import { expect, test, type Page } from "@playwright/test";

const open = (page: Page, state = "paired", section = "sync") => page.goto(`/?platform=win&sync=${state}#/settings/${section}`);
const mock = (page: Page, fn: string, ...args: unknown[]) => page.evaluate(([f, a]) => (window as unknown as { __ghiMock: Record<string, (...x: unknown[]) => void> }).__ghiMock[f as string]!(...(a as unknown[])), [fn, args] as const);

test("is off until switched on; then pairing and the local-network note show", async ({ page }) => {
  await open(page, "off");
  const toggle = page.getByRole("switch", { name: "Sync with my phone" });
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await expect(page.getByRole("button", { name: "Pair a phone" })).toHaveCount(0);
  await toggle.click();
  await expect(page.getByRole("button", { name: "Pair a phone" })).toBeVisible();
  await expect(page.getByText("Local network only. Nothing goes through the internet.")).toBeVisible();
  await expect(page.getByText("Can’t find your phone?")).toBeVisible();
});

test("the pair sheet shows a code under the prod CSP, expires after 120 s and shows a new one", async ({ page }) => {
  await page.clock.install();
  await open(page, "pairing");
  await page.getByRole("button", { name: "Pair a phone" }).click();
  const dialog = page.getByRole("dialog", { name: "Pair a phone" });
  const qr = dialog.getByRole("img", { name: "Pairing code for your phone" });
  await expect(qr).toBeVisible();
  expect(await qr.evaluate((i: HTMLImageElement) => i.complete && i.naturalWidth > 0)).toBe(true);
  await expect(dialog.getByTestId("pair-countdown")).toContainText("This code works for 2:00 more.");
  await page.clock.fastForward(60_000);
  await expect(dialog.getByTestId("pair-countdown")).toContainText("1:0");
  await page.clock.fastForward(61_000);
  await expect(dialog.getByText("This code has expired.")).toBeVisible();
  await expect(qr).toHaveCount(0);
  await dialog.getByRole("button", { name: "Show a new code" }).click();
  await expect(qr).toBeVisible();
  await expect(dialog.getByTestId("pair-countdown")).toContainText("2:00");
});

test("Regenerate while a code is showing, then Paired names the phone and offers Unpair", async ({ page }) => {
  await open(page, "pairing");
  await page.getByRole("button", { name: "Pair a phone" }).click();
  const dialog = page.getByRole("dialog", { name: "Pair a phone" });
  await expect(dialog.getByRole("img", { name: "Pairing code for your phone" })).toBeVisible();
  await dialog.getByRole("button", { name: "Show a new code" }).click();
  await expect(dialog.getByRole("img", { name: "Pairing code for your phone" })).toBeVisible();
  await mock(page, "syncSimulatePaired");
  const paired = page.getByRole("dialog", { name: "Paired with iPhone 16" });
  await expect(paired).toBeVisible();
  await paired.getByRole("button", { name: "Unpair" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByText("No phone paired yet.")).toBeVisible();
});

test("Escape closes the pair sheet and returns focus to its button", async ({ page }) => {
  await open(page, "pairing");
  const opener = page.getByRole("button", { name: "Pair a phone" });
  await opener.click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(opener).toBeFocused();
});

test("a paired phone: synced time, voice profiles stay local, and Unpair asks first", async ({ page }) => {
  await open(page);
  const row = page.getByTestId("device-device-iphone");
  await expect(row).toContainText("iPhone 16");
  await expect(row).toContainText(/Synced 4 min/);
  await expect(page.getByText("Voice profiles stay on each device.")).toBeVisible();
  await row.getByRole("button", { name: "Unpair", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Unpair iPhone 16?" });
  await expect(dialog).toContainText("Meetings already on each one stay where they are.");
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(row).toBeVisible();
  await row.getByRole("button", { name: "Unpair", exact: true }).click();
  await dialog.getByRole("button", { name: "Unpair", exact: true }).click();
  await expect(page.getByText("No phone paired yet.")).toBeVisible();
});

test("Unpair and wipe says what a wipe cannot do, then waits for the phone", async ({ page }) => {
  await open(page);
  await page.getByTestId("device-device-iphone").getByRole("button", { name: "Unpair and wipe" }).click();
  const dialog = page.getByRole("dialog", { name: "Unpair iPhone 16 and wipe it?" });
  await expect(dialog).toContainText("anyone who can unlock that phone and open Ghira can still read what is on it");
  await dialog.getByRole("button", { name: "Unpair and wipe" }).click();
  await expect(page.getByText("Waiting to wipe. Open Ghira on iPhone 16 to finish.")).toBeVisible();
  await mock(page, "syncSimulateWipeDone");
  await expect(page.getByText("No phone paired yet.")).toBeVisible();
  await expect(page.getByText("iPhone 16 deleted its synced meetings.").first()).toBeVisible();
});

test("a failed session is worded, pending work asks to open the phone, hotspot guidance and an export button show", async ({ page }) => {
  await open(page, "error");
  await expect(page.getByTestId("sync-error")).toContainText("Couldn’t reach the other device");
  await expect(page.getByText("Open Ghira on your phone to sync.")).toBeVisible();
  await expect(page.getByText(/Personal Hotspot/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Export for another device" })).toBeEnabled();
});

test("Export for another device asks for a repeated passphrase and says what was saved", async ({ page }) => {
  await open(page, "error");
  const opener = page.getByRole("button", { name: "Export for another device" });
  await opener.click();
  const dialog = page.getByRole("dialog", { name: "Export for another device" });
  const save = dialog.getByRole("button", { name: "Save file…" });
  await expect(save).toBeDisabled();
  await dialog.getByLabel("Passphrase", { exact: true }).fill("correct horse");
  await dialog.getByLabel("Repeat the passphrase").fill("correct horse");
  await expect(save).toBeEnabled();
  await save.click();
  await expect(page.getByText("Saved Ghira transfer 2026-10-06.ghix with 3 meetings", { exact: true })).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(opener).toBeFocused();
});

test("Import from another device words a wrong passphrase, then imports and says what was left out", async ({ page }) => {
  await open(page, "off");
  await page.getByRole("button", { name: "Import from another device" }).click();
  const dialog = page.getByRole("dialog", { name: "Import from another device" });
  await dialog.getByLabel("Passphrase").fill("correct horse");
  await mock(page, "syncTransferNext", "wrongPassphrase");
  await dialog.getByRole("button", { name: "Choose file…" }).click();
  await expect(dialog.getByText(/doesn’t open this file/)).toBeVisible();
  await mock(page, "syncTransferNext", "refused");
  await dialog.getByRole("button", { name: "Choose file…" }).click();
  await expect(page.getByText("Imported 3 meetings from Ghira transfer 2026-10-06.ghix", { exact: true })).toBeVisible();
  await expect(page.getByText("1 meeting in the file was deleted here earlier, so it was left out.", { exact: true })).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("another device's mass delete waits for an answer and Not now holds it", async ({ page }) => {
  await open(page);
  await expect(page.getByTestId("device-device-iphone")).toBeVisible();
  await mock(page, "syncSet", "needsConfirm");
  const dialog = page.getByRole("dialog", { name: "iPhone 16 deleted 12 meetings" });
  await expect(dialog).toContainText("Nothing is deleted until you choose, and everything else keeps syncing.");
  await page.keyboard.press("Escape");
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Not now" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("the conflict banner on a meeting: Use this applies the copy and the banner goes", async ({ page }) => {
  await page.goto("/?platform=win&sync=conflict#/meetings");
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();
  const banner = page.getByTestId("conflict-banner");
  await expect(banner).toContainText("Edited on iPhone 16");
  await banner.getByRole("button", { name: "Use this" }).click();
  await expect(banner).toHaveCount(0);
});

test("the conflict banner: Dismiss leaves the meeting as it is", async ({ page }) => {
  await page.goto("/?platform=win&sync=conflict#/meetings");
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();
  await page.getByTestId("conflict-banner").getByRole("button", { name: "Dismiss" }).click();
  await expect(page.getByTestId("conflict-banner")).toHaveCount(0);
});

test("deleting a meeting with a paired phone says where it is deleted, then on all devices", async ({ page }) => {
  await page.clock.install();
  await page.goto("/?platform=win&sync=paired#/meetings");
  const row = page.getByRole("listitem").filter({ hasText: "Client call — Acme onboarding" });
  await row.getByRole("button", { name: /Client call/ }).focus();
  await row.getByRole("button", { name: "Delete" }).click();
  await row.getByRole("button", { name: "Delete" }).last().click();
  await page.clock.fastForward(7000);
  await expect(page.getByText("Deleted here. It will be deleted on iPhone 16 at the next sync.").first()).toBeVisible();
  await mock(page, "syncSimulateProgress", 0);
  await expect(page.getByText("Deleted on all devices").first()).toBeVisible();
});

test("delete everything asks about paired devices and shows the wait for an unreachable one", async ({ page }) => {
  await open(page, "paired", "privacy");
  await page.getByRole("button", { name: "Delete all meetings and voice data…" }).click();
  await page.getByLabel("Type DELETE to confirm").fill("DELETE");
  await page.getByRole("button", { name: "Delete everything" }).click();
  const ask = page.getByRole("group", { name: "Also delete on your paired devices?" });
  await expect(ask).toContainText("iPhone 16");
  await ask.getByRole("button", { name: "Delete everywhere" }).click();
  await mock(page, "syncSetDeleteEverywhere", "waiting", ["iPhone 16"]);
  const waiting = page.getByTestId("delete-waiting");
  await expect(waiting).toContainText("Waiting for iPhone 16…");
  await expect(waiting.getByRole("button", { name: "Delete here only" })).toBeVisible();
});

test("Privacy repeats that sync is local only when it is on", async ({ page }) => {
  await open(page, "off", "privacy");
  await expect(page.getByTestId("strict-offline")).toBeVisible();
  await expect(page.getByTestId("strict-offline-local-note")).toHaveCount(0);
  await open(page, "paired", "privacy");
  await expect(page.getByTestId("strict-offline-local-note")).toContainText("Local network only");
});
