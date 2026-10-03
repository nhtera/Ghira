// SPDX-License-Identifier: Apache-2.0
// The 16-J visual matrix: light and dark, English and Vietnamese, and 200%
// text. Each cell checks axe and compares against a baseline.
import { expect, test, type Page } from "@playwright/test";
import { expectAccessible, openApp } from "./helpers";

export const VARIANTS = [
  { name: "en-light", lang: "en", scheme: "light", scale: undefined },
  { name: "en-dark", lang: "en", scheme: "dark", scale: undefined },
  { name: "vi-light", lang: "vi", scheme: "light", scale: undefined },
  { name: "vi-dark", lang: "vi", scheme: "dark", scale: undefined },
  { name: "en-200", lang: "en", scheme: "light", scale: 2 },
] as const;

export type Screen = {
  name: string;
  route: string;
  /** Gets the screen into the state to compare (after the app opened on `route`). */
  setup?: (page: Page) => Promise<void>;
  /** Called with the page once the screen is ready, to wait for what matters. */
  ready: (page: Page) => Promise<void>;
};

/** One test per screen and variant: opens it, waits, checks axe, compares to the baseline. */
export function visualMatrix(group: string, screens: Screen[]) {
  for (const v of VARIANTS) {
    test.describe(`${group} ${v.name}`, () => {
      // 1x keeps the baselines small; the layout is the same at any pixel ratio.
      test.use({ colorScheme: v.scheme, deviceScaleFactor: 1 });
      for (const s of screens) {
        test(s.name, async ({ page }) => {
          await openApp(page, "/settings", { lang: v.lang, scale: v.scale });
          if (s.setup) await s.setup(page);
          await page.evaluate((r) => (location.hash = `#${r}`), s.route);
          await s.ready(page);
          // Clicks scroll the control into view, and how far depends on timing at large text:
          // compare every cell from the top.
          await page.evaluate(() => {
            for (const el of document.querySelectorAll("*")) if (el.scrollTop > 0) el.scrollTop = 0;
          });
          await page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));
          await expectAccessible(page);
          await expect(page).toHaveScreenshot(`${group}-${s.name}-${v.name}.png`);
        });
      }
    });
  }
}
