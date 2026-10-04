// SPDX-License-Identifier: Apache-2.0
// Visual baselines for M3 / M4 / search: EN and VI, light and dark, and 200% text.
import { expect, test, type Page } from "@playwright/test";
import { openMeetings, type Opts } from "./meetings-helpers";

const variants: [string, Opts][] = [
  ["en-light", { lang: "en" }],
  ["en-dark", { lang: "en", dark: true }],
  ["vi-light", { lang: "vi" }],
  ["vi-dark", { lang: "vi", dark: true }],
  ["en-200", { lang: "en", scale: 2 }],
  ["vi-200", { lang: "vi", scale: 2 }],
];

async function settled(page: Page, ready: () => Promise<void>) {
  await ready();
  await page.evaluate(() => document.fonts.ready);
}

test.describe("meetings visuals", () => {
  for (const [name, opts] of variants) {
    test(`list ${name}`, async ({ page }) => {
      await openMeetings(page, "/meetings", opts);
      await settled(page, () =>
        expect(page.locator('[data-meeting="m-old2"]')).toBeVisible(),
      );
      await expect(page).toHaveScreenshot(`list-${name}.png`);
    });

    test(`meeting notes ${name}`, async ({ page }) => {
      await openMeetings(page, "/meetings/m-notes", opts);
      await settled(page, () =>
        expect(page.getByRole("tab", { selected: true })).toBeVisible(),
      );
      await expect(page).toHaveScreenshot(`notes-${name}.png`);
    });

    test(`meeting transcript ${name}`, async ({ page }) => {
      await openMeetings(page, "/meetings/m-notes", opts);
      // (With a ?lang= in the page URL the router drops the hash query, so pick the tab.)
      await page.getByRole("tab", { name: /Transcript|Bản ghi/ }).click();
      await settled(page, () =>
        expect(page.locator("[data-segment]").first()).toBeVisible(),
      );
      await expect(page).toHaveScreenshot(`transcript-${name}.png`);
    });

    test(`quote sheet ${name}`, async ({ page }) => {
      await openMeetings(page, "/meetings/m-notes", opts);
      await page.getByRole("button", { name: /01:30/ }).first().click();
      await settled(page, () => expect(page.getByRole("dialog")).toBeVisible());
      await expect(page).toHaveScreenshot(`quote-${name}.png`);
    });

    test(`search ${name}`, async ({ page }) => {
      await openMeetings(page, "/search", opts);
      await page.getByRole("searchbox").fill("dong");
      await settled(page, () =>
        expect(page.locator("mark").first()).toBeVisible(),
      );
      await expect(page).toHaveScreenshot(`search-${name}.png`);
    });
  }

  test("empty list", async ({ page }) => {
    await openMeetings(page, "/meetings", { meetings: "empty" });
    await expect(
      page.getByRole("heading", { name: "No meetings yet" }),
    ).toBeVisible();
    await expect(page).toHaveScreenshot("list-empty.png");
  });

  test("meeting without notes", async ({ page }) => {
    await openMeetings(page, "/meetings/m-nonotes");
    await expect(
      page.getByRole("heading", { name: "Notes: not generated on this phone" }),
    ).toBeVisible();
    await expect(page).toHaveScreenshot("notes-empty.png");
  });
});
