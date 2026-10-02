// SPDX-License-Identifier: Apache-2.0
// axe on every story (all states, overlays opened one by one) in light/dark ×
// en/vi: no serious or critical violations (phase 9 success criterion).
import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { VARIANTS, galleryEntries, storyUrl } from "./gallery";

test("axe: no serious violations in any story", async ({ page }) => {
  test.setTimeout(10 * 60_000);
  // Colors are checked at rest: a pulse caught mid-fade is not a contrast bug.
  await page.emulateMedia({ reducedMotion: "reduce" });
  const entries = await galleryEntries(page);
  const found: string[] = [];
  for (const e of entries) {
    for (const v of VARIANTS) {
      for (const state of e.states) {
        await page.goto(storyUrl(e.id, v, { state }));
        await page.waitForLoadState("load");
        const serious = async () => {
          // Settled: fonts in and two frames painted (a busy machine can be
          // caught mid-render; a real violation is still there on the retry).
          await page.evaluate(() => document.fonts.ready.then(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)))));
          const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
          return r.violations.filter((x) => x.impact === "serious" || x.impact === "critical");
        };
        let violations = await serious();
        if (violations.length) {
          await page.waitForTimeout(500);
          violations = await serious();
        }
        for (const x of violations) {
          found.push(`${e.id}/${state} ${v.theme}/${v.lang}: ${x.id} (${x.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
        }
      }
    }
  }
  expect(found).toEqual([]);
});
