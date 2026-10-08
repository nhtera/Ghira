// SPDX-License-Identifier: Apache-2.0

// Accessibility on the built site: axe (WCAG 2.2 AA + best practices) on the
// landing page, two docs pages and the 404 page, in both themes at four
// viewports, with no exemptions; no sideways scrolling from 320 px up; on a
// coarse pointer every control is at least 44×44 px (links inside running
// text excepted); the header's call to action stays on one line on a phone.

import AxeBuilder from "@axe-core/playwright";
import assert from "node:assert/strict";
import { test } from "node:test";
import { openPage, siteFixture } from "./helpers.mjs";

const site = siteFixture();
const PAGES = ["/", "/docs/getting-started", "/docs/cli", "/nope"];
const VIEWPORTS = [
  [375, 812],
  [768, 1024],
  [1024, 768],
  [1440, 900],
];
const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa", "best-practice"];

for (const path of PAGES) {
  for (const theme of ["light", "dark"]) {
    test(`axe: ${path} ${theme}`, async () => {
      const problems = [];
      for (const [width, height] of VIEWPORTS) {
        const { page, context } = await openPage(site, path, { width, height, theme, touch: width < 800 });
        const { violations } = await new AxeBuilder({ page }).withTags(TAGS).analyze();
        for (const v of violations) problems.push(`${width}px ${v.id} (${v.impact}): ${v.nodes.length}× ${v.nodes.slice(0, 3).map((n) => n.target.join(" ")).join(" | ")}`);
        await context.close();
      }
      assert.deepEqual(problems, []);
    });
  }
}

test("no sideways scrolling at 320, 375, 390, 768 and 1280 px", async () => {
  const wide = [];
  for (const path of PAGES) {
    for (const width of [320, 375, 390, 768, 1280]) {
      const { page, context } = await openPage(site, path, { width, height: 800, touch: width < 800 });
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
      if (overflow > 0) wide.push(`${path} at ${width}px: ${overflow}px too wide`);
      await context.close();
    }
  }
  assert.deepEqual(wide, []);
});

test("coarse pointer: controls are at least 44×44 px", async () => {
  const small = [];
  for (const path of ["/", "/docs/getting-started", "/nope"]) {
    const { page, context } = await openPage(site, path, { width: 390, height: 844, touch: true });
    const found = await page.evaluate(() => {
      const out = [];
      const inProse = (el) => el.tagName === "A" && el.closest("p, li, td, figcaption, .article, .band-lead, .footnote, .get-note, .compare-sources") && !el.closest("nav, .foot-col, .pager, .results");
      for (const el of document.querySelectorAll("a, button, summary, [role=tab]")) {
        const s = getComputedStyle(el);
        if (s.display === "none" || s.visibility === "hidden" || el.closest("[hidden], [aria-hidden=true], dialog:not([open])")) continue;
        if (inProse(el)) continue;
        const r = el.getBoundingClientRect();
        if (r.width === 0 && r.height === 0) continue;
        // A citation chip's hit area is extended by its ::after.
        const after = getComputedStyle(el, "::after");
        let h = r.height;
        let w = r.width;
        if (after.content !== "none" && after.position === "absolute") {
          h += -parseFloat(after.top) - parseFloat(after.bottom);
          w += -parseFloat(after.left) - parseFloat(after.right);
        }
        if (h < 44 - 0.5 || w < 44 - 0.5) out.push(`${el.tagName.toLowerCase()}.${el.className || ""} "${(el.textContent || el.getAttribute("aria-label") || "").trim().slice(0, 30)}" ${Math.round(w)}×${Math.round(h)}`);
      }
      return out;
    });
    small.push(...found.map((f) => `${path}: ${f}`));
    await context.close();
  }
  assert.deepEqual(small, []);
});

test("the header call to action stays on one line at 375 px", async () => {
  const { page, context } = await openPage(site, "/", { width: 375, height: 812, touch: true });
  const lines = await page.locator(".head-cta").evaluate((el) => Math.round(el.getBoundingClientRect().height / parseFloat(getComputedStyle(el).lineHeight || "20")));
  const { height } = await page.locator(".head-cta").boundingBox();
  assert.ok(height <= 48, `CTA is ${height}px tall (${lines} lines)`);
  await context.close();
});
