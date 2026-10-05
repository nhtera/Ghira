// SPDX-License-Identifier: Apache-2.0
// The tab bar is flush with the bottom edge and takes the home-indicator inset
// exactly once. Playwright has no safe areas, so the iPhone values (top 59,
// bottom 34) are put on the CSS variables the app reads (--safe-*, which are
// env(safe-area-inset-*) on a device). The web view itself is kept full height
// by native/ios/GhiAudio/WebViewInsets.swift (checked on the simulator).
import { expect, test } from "@playwright/test";
import { openApp } from "./helpers";

for (const route of ["/meetings", "/record", "/search", "/settings"]) {
  test(`tab bar sits on the bottom edge with one home-indicator inset: ${route}`, async ({ page }) => {
    await openApp(page, route);
    await page.evaluate(() => {
      document.documentElement.style.setProperty("--safe-top", "59px");
      document.documentElement.style.setProperty("--safe-bottom", "34px");
    });
    const nav = page.getByRole("navigation", { name: /tabs|thanh/i });
    await expect(nav).toBeVisible();
    const m = await nav.evaluate((el) => {
      const r = el.getBoundingClientRect();
      const first = el.querySelector("button")!.getBoundingClientRect();
      return { bottom: r.bottom, vh: window.innerHeight, pad: parseFloat(getComputedStyle(el).paddingBottom), buttonBottom: first.bottom, height: r.height };
    });
    expect(m.bottom).toBeCloseTo(m.vh, 0);
    expect(m.pad).toBe(34);
    // The tabs end above the inset, the inset is the only space below them.
    expect(m.bottom - m.buttonBottom).toBeGreaterThanOrEqual(34);
    expect(m.bottom - m.buttonBottom).toBeLessThan(34 + 12);
  });
}
