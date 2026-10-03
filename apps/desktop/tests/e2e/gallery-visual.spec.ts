// SPDX-License-Identifier: Apache-2.0
// One contact sheet per story × theme × language (WebKit, macOS only: fonts
// render differently elsewhere). Update with `--update-snapshots` after an
// intended change; review the PNG diffs.
import { expect, test } from "@playwright/test";
import { VARIANTS, galleryEntries, isPhone, storyUrl } from "./gallery";

test.skip(process.platform !== "darwin", "visual baselines are macOS WebKit");

test("contact sheets", async ({ page }) => {
  test.setTimeout(10 * 60_000);
  const entries = await galleryEntries(page);
  // The phone's stories have their own project (gallery-visual-ios).
  for (const e of entries.filter((e) => !isPhone(e))) {
    for (const v of VARIANTS) {
      await page.goto(storyUrl(e.id, v));
      await page.waitForLoadState("load");
      await page.evaluate(() => document.fonts.ready);
      await expect.soft(page).toHaveScreenshot(`${e.id}-${v.theme}-${v.lang}.png`, { fullPage: true, animations: "disabled" });
    }
  }
});
