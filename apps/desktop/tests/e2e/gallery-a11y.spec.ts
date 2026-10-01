// SPDX-License-Identifier: Apache-2.0
// axe on every story (all states, overlays opened one by one) in light/dark ×
// en/vi: no serious or critical violations (phase 9 success criterion).
import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { VARIANTS, galleryEntries, storyUrl } from "./gallery";

test("axe: no serious violations in any story", async ({ page }) => {
  test.setTimeout(10 * 60_000);
  const entries = await galleryEntries(page);
  const found: string[] = [];
  for (const e of entries) {
    for (const v of VARIANTS) {
      for (const state of e.states) {
        await page.goto(storyUrl(e.id, v, { state }));
        await page.waitForLoadState("load");
        const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
        for (const x of r.violations.filter((x) => x.impact === "serious" || x.impact === "critical")) {
          found.push(`${e.id}/${state} ${v.theme}/${v.lang}: ${x.id} (${x.nodes.map((n) => n.target.join(" ")).slice(0, 3).join(", ")})`);
        }
      }
    }
  }
  expect(found).toEqual([]);
});
