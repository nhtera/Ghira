// SPDX-License-Identifier: Apache-2.0
// The website's phone screenshots (apps/website, `npm run screens` turns them
// into the site's WebP). A dev tool, not a gate: skipped unless GHI_MARKETING=1.
//   GHI_MARKETING=1 pnpm --filter @ghi/mobile exec playwright test marketing
// Writes <id>-<theme>.png (1179x2556 = 393x852 @3x) to $GHI_MARKETING_OUT
// (default apps/website/.screens). Deterministic: seeded Math.random, a fake
// clock, reduced motion, one fresh context per shot and theme.
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { devices, expect, test, type Browser, type Page } from "@playwright/test";
import { openApp, settle } from "./helpers";
import { startRecording } from "./record-support";

test.skip(!process.env.GHI_MARKETING, "set GHI_MARKETING=1 to capture the website screenshots");

const OUT = resolve(process.env.GHI_MARKETING_OUT ?? resolve(process.cwd(), "../website/.screens"));
const THEMES = ["light", "dark"] as const;
type Theme = (typeof THEMES)[number];

/** A fresh context and page: iPhone size, theme, seeded randomness, a fake clock, console errors collected. */
async function shot(browser: Browser, baseURL: string, theme: Theme) {
  const { defaultBrowserType: _browser, ...phone } = devices["iPhone 15 Pro"]; // 393x852 @3x, touch
  const context = await browser.newContext({ ...phone, viewport: { width: 393, height: 852 }, baseURL, colorScheme: theme, reducedMotion: "reduce", locale: "en-US" });
  const page = await context.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  // The test server has no CSP nonces (Tauri adds them), so the inline <style> of a
  // Radix sheet's scroll lock is refused here: a server artefact, not an app error.
  const nonceArtefact = /Refused to apply a stylesheet/;
  page.on("console", (m) => m.type() === "error" && !nonceArtefact.test(m.text()) && errors.push(m.text()));
  await page.addInitScript(() => {
    let s = 12345; // an LCG: the same "random" numbers every run
    Math.random = () => ((s = (Math.imul(s, 1664525) + 1013904223) >>> 0) / 2 ** 32);
  });
  await page.clock.install({ time: new Date("2026-10-09T09:30:00Z") });
  return { context, page, errors };
}

/** A row title still fading in (a colour transition `settle` cannot see) is near-invisible in a shot:
 * wait until every element's colours have held for 15 frames in a row (at most ~4 s). */
async function coloursStill(page: Page) {
  await page.evaluate(async () => {
    const frame = () => new Promise<void>((r) => requestAnimationFrame(() => r()));
    const colours = () => [...document.querySelectorAll("body *")].map((el) => getComputedStyle(el).color + getComputedStyle(el).opacity).join("|");
    let last = colours();
    for (let still = 0, n = 0; still < 15 && n < 240; n++) {
      await frame();
      const now = colours();
      still = now === last ? still + 1 : 0;
      last = now;
    }
  });
}

async function capture(page: Page, errors: string[], id: string, theme: Theme) {
  await settle(page);
  await coloursStill(page);
  expect(await page.evaluate(() => document.documentElement.dataset.theme)).toBe(theme);
  // Checked before the screenshot: Playwright injects a caret-hiding stylesheet the prod CSP refuses.
  expect(errors).toEqual([]);
  mkdirSync(OUT, { recursive: true });
  const png = await page.screenshot({ path: resolve(OUT, `${id}-${theme}.png`) });
  expect({ w: png.readUInt32BE(16), h: png.readUInt32BE(20) }).toEqual({ w: 1179, h: 2556 });
}

for (const theme of THEMES) {
  test(`phone-live ${theme}`, async ({ browser, baseURL }) => {
    const { context, page, errors } = await shot(browser, baseURL!, theme);
    await openApp(page, "/", { lang: "en" });
    await page.evaluate(() => {
      Object.assign(window.__ghiRecord as object, { firstIsMe: true });
      window.location.hash = "#/record";
    });
    await expect(page.locator("[data-screen=record]")).toBeVisible();
    await startRecording(page, /record room/i);
    await expect(page.getByRole("button", { name: /^Stop$/ })).toBeVisible();
    await expect(page.getByText(/Getting ready/)).toHaveCount(0);
    // Four lines, never five: the fifth sample is a diacritics test string.
    await page.evaluate(() => window.__ghiRecord!.addLines(4));
    await expect(page.getByTestId("line")).toHaveCount(4);
    await page.evaluate(() => window.__ghiRecord!.addPartial("Vậy mình chốt lại scope cho"));
    await expect(page.getByText("Vậy mình chốt lại scope cho")).toBeVisible();
    await page.clock.runFor(47_000);
    await expect(page.getByText(/00:4\d/).first()).toBeVisible();
    // Put every scroll area at its newest line before the shot.
    await settle(page);
    await page.evaluate(() => {
      for (const el of document.querySelectorAll("*")) if (el.scrollHeight > el.clientHeight) el.scrollTop = el.scrollHeight;
    });
    await capture(page, errors, "phone-live", theme);
    await context.close();
  });

  test(`phone-meetings ${theme}`, async ({ browser, baseURL }) => {
    const { context, page, errors } = await shot(browser, baseURL!, theme);
    await openApp(page, "/meetings", { lang: "en" });
    await expect(page.locator("[data-screen=meetings]")).toBeVisible();
    await capture(page, errors, "phone-meetings", theme);
    await context.close();
  });
}
