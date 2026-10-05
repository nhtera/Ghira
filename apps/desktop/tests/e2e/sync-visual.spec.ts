// SPDX-License-Identifier: Apache-2.0
// Phase 15-G: axe on Settings > Sync and the pair sheet, and macOS WebKit
// baselines for both, light and dark, English and Vietnamese.
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

const VARIANTS = [
  { theme: "light", language: "en" },
  { theme: "dark", language: "en" },
  { theme: "light", language: "vi" },
  { theme: "dark", language: "vi" },
] as const;

async function show(page: Page, v: (typeof VARIANTS)[number], what: "section" | "sheet") {
  await page.setViewportSize({ width: 1280, height: 860 });
  await page.addInitScript((state) => localStorage.setItem("ghira.prefs", JSON.stringify({ state, version: 0 })), v);
  await page.goto(`/?platform=mac&sync=${what === "section" ? "error" : "pairing"}#/settings/sync`);
  const pair = page.getByRole("button", { name: v.language === "en" ? "Pair a phone" : "Ghép nối điện thoại" });
  await expect(pair).toBeVisible();
  if (what === "sheet") {
    await pair.click();
    await expect(page.getByTestId("pair-qr").getByRole("img")).toBeVisible();
  }
}

const AXE = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];

for (const v of VARIANTS)
  for (const what of ["section", "sheet"] as const) {
    const name = `${what}-${v.theme}-${v.language}`;
    test(`axe: sync ${name}`, async ({ page, browserName }) => {
      test.skip(browserName !== "chromium", "axe runs once, in chromium");
      await show(page, v, what);
      const r = await new AxeBuilder({ page }).withTags(AXE).analyze();
      expect(r.violations.map((x) => `${x.id}: ${x.nodes.map((n) => n.target).join(" ")}`)).toEqual([]);
    });

    test(`visual baseline: sync ${name}`, async ({ page, browserName }) => {
      test.skip(browserName !== "webkit" || process.platform !== "darwin", "WKWebView on macOS is what ships; CI has no baselines");
      await show(page, v, what);
      await expect(page).toHaveScreenshot(`sync-${name}.png`, { animations: "disabled" });
    });
  }
