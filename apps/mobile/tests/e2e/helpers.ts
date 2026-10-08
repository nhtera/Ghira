// SPDX-License-Identifier: Apache-2.0
// Shared helpers for the mobile e2e specs.
import AxeBuilder from "@axe-core/playwright";
import { expect, type Page } from "@playwright/test";

/** Opens a route on the scripted mock once its hooks are ready. */
export async function openApp(page: Page, route = "/", opts: { lang?: "en" | "vi"; scale?: number } = {}) {
  const q = new URLSearchParams();
  if (opts.lang) q.set("lang", opts.lang);
  if (opts.scale) q.set("scale", String(opts.scale));
  await page.goto(`/${q.size ? `?${q}` : ""}#${route}`);
  await page.waitForFunction(() => Boolean(window.__ghiMock));
}

/** Waits for every finite CSS animation and transition (a sheet sliding in, a
 * fade) to end, then for smooth scrolling (the live transcript following its
 * newest line) to stop, so axe and screenshots see the settled screen: a slow
 * CI machine is still mid-transition or mid-scroll otherwise. */
export async function settle(page: Page) {
  await page.evaluate(async () => {
    await Promise.all(
      document
        .getAnimations()
        .filter((a) => a.effect?.getComputedTiming().iterations !== Infinity)
        .map((a) => a.finished.catch(() => undefined)),
    );
    await document.fonts.ready;
    const frame = () => new Promise<void>((r) => requestAnimationFrame(() => r()));
    // Scroll positions and content heights (a virtual list measures its rows
    // after they mount) still for 5 frames in a row (at most ~2 s).
    const scrolls = () => [...document.querySelectorAll("*")].map((el) => `${el.scrollTop}/${el.scrollHeight}`).join(",");
    let last = scrolls();
    for (let still = 0, n = 0; still < 5 && n < 120; n++) {
      await frame();
      const now = scrolls();
      still = now === last ? still + 1 : 0;
      last = now;
    }
  });
}

/** No axe violations on the current screen. */
export async function expectAccessible(page: Page) {
  await settle(page);
  const r = await new AxeBuilder({ page }).analyze();
  expect(r.violations.map((v) => `${v.id}: ${v.nodes.length}`)).toEqual([]);
}
