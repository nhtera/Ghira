// SPDX-License-Identifier: Apache-2.0
// 15-I: sync in the meeting view and the list on the scripted mock: the chips
// (Synced, Waiting for Wi-Fi, final pass on the computer with its %), audio on
// the computer, the read-only transcript while a final pass is open ("Process
// on this phone now" takes it back), and the conflict banner.
import { expect, test } from "@playwright/test";
import { expectAccessible } from "./helpers";
import { recordPlatform } from "./meetings-helpers";
import { openSync } from "./sync-support";

test.beforeEach(async ({ page }) => {
  await recordPlatform(page);
});

const NOTES = "/meetings/m-notes";

// A query string in front of the hash drops the route's own ?tab=, so pick the tab in the UI.
async function openTranscript(page: import("@playwright/test").Page, sync: string, lang?: "en" | "vi") {
  await openSync(page, NOTES, { sync, lang });
  await page.getByRole("tab", { name: lang === "vi" ? "Bản ghi" : "Transcript" }).click();
}

test.describe("chips", () => {
  for (const [sync, kind, text] of [
    ["desktopAudio", "synced", "Synced"],
    ["waiting", "waitingForWifi", "Waiting for Wi-Fi"],
    ["leased", "finalOnDesktop", "Final pass on MacBook Pro · 42%"],
  ] as const) {
    test(`${kind} in the list and on the meeting`, async ({ page }) => {
      await openSync(page, "/meetings", { sync });
      const row = page.locator('[data-meeting="m-notes"]');
      await expect(row.locator(`[data-chip="${kind}"]`)).toContainText(kind === "finalOnDesktop" ? /Final pass on .* · 42%/ : text);
      await page.goto(`/?sync=${sync}#${NOTES}`);
      const chip = page.locator(`[data-screen=meeting] [data-chip="${kind}"]`);
      await expect(chip).toBeVisible();
      // The meeting names the computer; the list's chip still says "My computer" (meeting-row has no device yet).
      if (kind === "finalOnDesktop") await expect(chip).toContainText("Final pass on MacBook Pro · 42%");
      await expectAccessible(page);
    });
  }
});

test("audio on the computer: the bar says where, with no player", async ({ page }) => {
  await openSync(page, NOTES, { sync: "desktopAudio" });
  const bar = page.getByTestId("audio-on-device");
  await expect(bar).toContainText("Audio is on MacBook Pro");
  await expect(bar).toContainText("Play it there. This phone has the notes and the transcript.");
  await expect(page.getByTestId("audio-seek")).toHaveCount(0);
  await expectAccessible(page);
  // Not paired: the meeting has its own audio, so the usual bar shows.
  await openSync(page, NOTES, { sync: "paired" });
  await expect(page.getByTestId("audio-on-device")).toHaveCount(0);
});

test.describe("a final pass open on the computer", () => {
  test("the transcript is read-only with Refining on <device>", async ({ page }) => {
    await openTranscript(page, "leased");
    const banner = page.getByRole("status").filter({ hasText: "Refining on MacBook Pro" });
    await expect(banner).toContainText("The transcript can’t be edited until it comes back.");
    await expectAccessible(page);
    // A line can be read and selected, never edited.
    const line = page.locator("[data-segment]").first();
    await line.click();
    await expect(page.getByRole("button", { name: "Edit line" })).toHaveCount(0);
  });

  test("Process on this phone now asks first, then takes the job back", async ({ page }) => {
    await openTranscript(page, "leased");
    await page.getByRole("button", { name: "Process on this phone now" }).click();
    const sheet = page.getByRole("dialog", { name: "Process on this phone now?" });
    await expect(sheet).toContainText("MacBook Pro stops refining this meeting and this phone takes over.");
    await expectAccessible(page);
    await sheet.getByRole("button", { name: "Cancel" }).click();
    await expect(sheet).toHaveCount(0);
    expect(await page.evaluate(() => (window as never as { __ghiMock: { revoked?: string[] } }).__ghiMock.revoked ?? [])).toEqual([]);

    await page.getByRole("button", { name: "Process on this phone now" }).click();
    await page.getByRole("dialog").getByRole("button", { name: "Process here" }).click();
    await expect(page.getByRole("status").filter({ hasText: "Refining on MacBook Pro" })).toHaveCount(0);
    await expect(page.locator('[data-screen=meeting] [data-chip="synced"]')).toBeVisible();
    // Editing works again.
    await page.locator("[data-segment]").first().click();
    await expect(page.getByRole("button", { name: "Edit line" })).toBeVisible();
  });

  test("Vietnamese copy", async ({ page }) => {
    await openTranscript(page, "leased", "vi");
    await expect(page.getByRole("status").filter({ hasText: "Đang xử lý cuối trên MacBook Pro" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Xử lý ngay trên điện thoại này" })).toBeVisible();
    await expect(page.getByTestId("audio-on-device")).toContainText("Âm thanh nằm trên MacBook Pro");
  });
});

test.describe("conflict copy", () => {
  test("shows who edited and the other text; Use this keeps it", async ({ page }) => {
    await openTranscript(page, "conflict");
    const banner = page.getByTestId("conflict-banner");
    await expect(banner).toContainText("Edited on MacBook Pro");
    await expect(banner).toContainText("Chốt scope cho bản beta vào thứ Sáu.");
    await expectAccessible(page);
    for (const name of ["Use this", "Dismiss"]) {
      expect((await banner.getByRole("button", { name }).boundingBox())!.height).toBeGreaterThanOrEqual(44);
    }
    await banner.getByRole("button", { name: "Use this" }).click();
    await expect(banner).toHaveCount(0);
  });

  test("Dismiss drops it", async ({ page }) => {
    await openSync(page, NOTES, { sync: "conflict" });
    await page.getByTestId("conflict-banner").getByRole("button", { name: "Dismiss" }).click();
    await expect(page.getByTestId("conflict-banner")).toHaveCount(0);
  });

  test("no conflict, no banner", async ({ page }) => {
    await openSync(page, NOTES, { sync: "paired" });
    await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
    await expect(page.getByTestId("conflict-banner")).toHaveCount(0);
  });

  test("a copy that arrives while the meeting is open shows up", async ({ page }) => {
    await openSync(page, NOTES, { sync: "paired" });
    await page.evaluate(() => window.__ghiMock!.syncSet("conflict"));
    await expect(page.getByTestId("conflict-banner")).toBeVisible();
  });
});
