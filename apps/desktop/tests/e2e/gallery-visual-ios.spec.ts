// SPDX-License-Identifier: Apache-2.0
// The phone's stories at 390 x 844 (platform=ios): light/dark x EN/VI, plus
// 200% text in Vietnamese. A story with no overlays is one contact sheet; an
// overlay (sheet) is shot open, state by state. WebKit on macOS only. Update
// with `--project gallery-visual-ios --update-snapshots` after an intended
// change; desktop baselines live in gallery-visual and are never touched here.
import { expect, test } from "@playwright/test";
import { PHONE_VARIANTS, galleryEntries, isPhone, phoneUrl } from "./gallery";

test.skip(process.platform !== "darwin", "visual baselines are macOS WebKit");

test("phone contact sheets", async ({ page }) => {
  test.setTimeout(15 * 60_000);
  const entries = (await galleryEntries(page)).filter(isPhone);
  expect(entries.length).toBeGreaterThan(0);
  for (const e of entries) {
    for (const v of PHONE_VARIANTS) {
      const shots = e.overlays.length ? e.states : [undefined];
      for (const state of shots) {
        await page.goto(phoneUrl(e.id, v, state ? { state } : {}));
        await page.waitForLoadState("load");
        await page.evaluate(() => document.fonts.ready);
        // A sheet's actions must stay reachable at every text scale: each button in
        // the footer is fully inside the viewport (never pushed off-screen).
        if (state && e.overlays.includes(state)) {
          // Measure after the slide-in has landed.
          await page.evaluate(() => Promise.all(document.getAnimations().filter((a) => a.effect?.getTiming().iterations !== Infinity).map((a) => a.finished)));
          const buttons = page.locator("[data-sheet-footer] button");
          expect(await buttons.count(), `${e.id}/${state} footer buttons`).toBeGreaterThan(0);
          const view = page.viewportSize()!;
          for (const box of await buttons.evaluateAll((els) => els.map((el) => el.getBoundingClientRect().toJSON()))) {
            expect(box.top, `${e.id}/${state} ${v.lang} x${v.scale}`).toBeGreaterThanOrEqual(0);
            expect(box.bottom, `${e.id}/${state} ${v.lang} x${v.scale}`).toBeLessThanOrEqual(view.height);
          }
        }
        const name = [e.id, state, v.theme, v.lang, v.scale === "1" ? null : `x${v.scale}`].filter(Boolean).join("-");
        await expect.soft(page).toHaveScreenshot(`${name}.png`, { fullPage: true, animations: "disabled" });
      }
    }
  }
});
