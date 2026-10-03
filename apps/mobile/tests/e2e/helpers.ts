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

/** No axe violations on the current screen. */
export async function expectAccessible(page: Page) {
  const r = await new AxeBuilder({ page }).analyze();
  expect(r.violations.map((v) => `${v.id}: ${v.nodes.length}`)).toEqual([]);
}
