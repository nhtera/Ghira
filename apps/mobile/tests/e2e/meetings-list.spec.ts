// SPDX-License-Identifier: Apache-2.0
// M3: the meetings list on the scripted mock.
import { expect, test } from "@playwright/test";
import { expectAccessible } from "./helpers";
import { expectRows, mock, openMeetings } from "./meetings-helpers";

test.describe("meetings list", () => {
  test("groups by day and shows a chip per state", async ({ page }) => {
    await openMeetings(page, "/meetings");
    await expect(
      page.getByRole("heading", { level: 1, name: "Meetings" }),
    ).toBeVisible();
    await expect(page.getByRole("heading", { name: "Today" })).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Yesterday" }),
    ).toBeVisible();
    await expectRows(page, ["m-proc", "m-nonotes", "m-fail"]);
    await expect(
      page.locator('[data-meeting="m-proc"] [data-chip]'),
    ).toHaveText("Processing on phone · 42%");
    await expect(
      page.locator('[data-meeting="m-nonotes"] [data-chip]'),
    ).toHaveText("Processed on phone");
    await expect(
      page.locator('[data-meeting="m-fail"] [data-chip]'),
    ).toHaveText("Failed · Tap to retry");
    await expect(
      page.locator('[data-meeting="m-notes"] [data-chip]'),
    ).toHaveText("Synced");
    await expect(
      page.locator('[data-meeting="m-old2"] [data-chip]'),
    ).toHaveText("Waiting for models");
    // Speakers are a color plus an initial.
    await expect(
      page
        .locator('[data-meeting="m-nonotes"]')
        .getByText("L", { exact: true }),
    ).toBeVisible();
  });

  test("live progress updates the percent", async ({ page }) => {
    await openMeetings(page, "/meetings");
    const chip = page.locator('[data-meeting="m-proc"] [data-chip]');
    await expect(chip).toHaveText("Processing on phone · 42%");
    await page.evaluate(() =>
      window.__ghiMock?.simulateCoreEvent({
        seq: 1,
        atMs: 1,
        event: {
          type: "jobProgress",
          meeting: "m-proc",
          job: 1,
          kind: "final_pass",
          stage: "decoding",
          progress: 0.7,
        },
      }),
    );
    await expect(chip).toHaveText("Processing on phone · 70%");
    // The phone's final pass ends: the row re-reads its chip.
    await page.evaluate(() =>
      window.__ghiMock?.simulateCoreEvent({
        seq: 2,
        atMs: 2,
        event: { type: "stateChanged", meeting: "m-proc", state: "ready" },
      }),
    );
    await expect(chip).toHaveText("Processing on phone · 42%");
  });

  test("a failed meeting retries from its chip", async ({ page }) => {
    await openMeetings(page, "/meetings");
    await page.locator('[data-meeting="m-fail"] [data-chip]').click();
    await expect(
      page.locator('[data-meeting="m-fail"] [data-chip]'),
    ).toHaveText("Processing on phone · 0%");
  });

  test("empty state offers recording", async ({ page }) => {
    await openMeetings(page, "/meetings", { meetings: "empty" });
    await expect(
      page.getByRole("heading", { name: "No meetings yet" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Start recording" }).click();
    await expect(page).toHaveURL(/#\/record$/);
  });

  test("1,200 meetings scroll without rendering them all", async ({ page }) => {
    await openMeetings(page, "/meetings", { meetings: "many" });
    await expectRows(page, ["g0"]);
    const scroller = page.locator('[data-screen="meetings"] > div').first();
    const last = page.locator('[data-meeting="g1199"]');
    for (let i = 0; i < 60 && !(await last.isVisible()); i++) {
      expect(await page.locator("[data-meeting]").count()).toBeLessThan(80);
      await scroller.evaluate((el) => el.scrollTo(0, el.scrollHeight));
      await page.waitForTimeout(100);
    }
    await expect(last).toBeVisible();
    await expect(
      last.getByRole("button", { name: /^Cuộc họp số 1200/ }),
    ).toBeVisible();
  });

  test("swipe actions: Delete asks first", async ({ page }) => {
    await openMeetings(page, "/meetings");
    const row = page.locator('[data-meeting="m-old2"]');
    await row.getByRole("button", { name: /^Delete/ }).focus();
    await row.getByRole("button", { name: /^Delete/ }).click();
    await expect(page.getByRole("alertdialog")).toContainText(
      "Delete “Weekly 1:1” and its audio from this phone?",
    );
    // Cancel keeps it.
    await page
      .getByRole("alertdialog")
      .getByRole("button", { name: "Cancel" })
      .click();
    await expect(row).toBeVisible();
    await row.getByRole("button", { name: /^Delete/ }).focus();
    await row.getByRole("button", { name: /^Delete/ }).click();
    await page
      .getByRole("alertdialog")
      .getByRole("button", { name: "Delete" })
      .click();
    await expect(row).toHaveCount(0);
  });

  test("a failed delete says so and keeps the meeting", async ({ page }) => {
    await openMeetings(page, "/meetings");
    await mock(page, "failDeletes", true);
    const row = page.locator('[data-meeting="m-old2"]');
    await row.getByRole("button", { name: /^Delete/ }).focus();
    await row.getByRole("button", { name: /^Delete/ }).click();
    await page
      .getByRole("alertdialog")
      .getByRole("button", { name: "Delete" })
      .click();
    await expect(
      page.getByText("Couldn’t delete the meeting. Try again."),
    ).toBeVisible();
    await expect(row).toBeVisible();
  });

  test("opens a meeting", async ({ page }) => {
    await openMeetings(page, "/meetings");
    await page
      .locator('[data-meeting="m-nonotes"]')
      .getByRole("button", { name: /^Họp kế hoạch/ })
      .click();
    await expect(page).toHaveURL(/#\/meetings\/m-nonotes$/);
  });

  test("is accessible, EN and VI, also at 200% text", async ({ page }) => {
    await openMeetings(page, "/meetings");
    await expectAccessible(page);
    await openMeetings(page, "/meetings", { lang: "vi", scale: 2 });
    await expect(page.getByRole("heading", { name: "Hôm nay" })).toBeVisible();
    await expectAccessible(page);
  });
});
