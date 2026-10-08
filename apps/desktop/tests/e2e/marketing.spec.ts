// SPDX-License-Identifier: Apache-2.0
// The website's desktop screenshots (apps/website, `npm run screens` turns them
// into the site's WebP). A dev tool, not a gate: skipped unless GHI_MARKETING=1.
//   GHI_MARKETING=1 pnpm --filter @ghi/desktop exec playwright test marketing --project=webkit
// Writes <id>-<theme>.png (2800x1602 = 1400x801 @2x) to $GHI_MARKETING_OUT
// (default apps/website/.screens). Deterministic: seeded Math.random, a fake
// clock stepped by hand (the mock's timers and Date agree), reduced motion,
// one fresh context per shot and theme.
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test, type Browser, type Page } from "@playwright/test";

test.skip(!process.env.GHI_MARKETING, "set GHI_MARKETING=1 to capture the website screenshots");
// WKWebView is what ships on macOS: one project, not one set of shots per browser.
test.skip(({ browserName }) => browserName !== "webkit", "the shots come from WebKit");

const OUT = resolve(process.env.GHI_MARKETING_OUT ?? resolve(process.cwd(), "../website/.screens"));
const THEMES = ["light", "dark"] as const;
const LINE_MS = 1800; // apps/desktop/src/ipc/mock.ts

type Theme = (typeof THEMES)[number];

/** Finite animations finished, fonts loaded, and scroll positions still for 5 frames (the mobile e2e helper's settle; desktop has none to import). */
async function settle(page: Page) {
  await page.evaluate(async () => {
    await Promise.all(
      document
        .getAnimations()
        .filter((a) => a.effect?.getComputedTiming().iterations !== Infinity)
        .map((a) => a.finished.catch(() => undefined)),
    );
    await document.fonts.ready;
    const frame = () => new Promise<void>((r) => requestAnimationFrame(() => r()));
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

/** A fresh context and page: theme, size, seeded randomness, a clock to step, console errors collected. */
async function shot(browser: Browser, theme: Theme) {
  const context = await browser.newContext({
    baseURL: "http://127.0.0.1:4173",
    viewport: { width: 1400, height: 801 },
    deviceScaleFactor: 2,
    colorScheme: theme,
    reducedMotion: "reduce",
    locale: "en-US",
    // The mock core plays a blob: WAV, which the production media-src refuses (as in meeting-transcript.spec.ts).
    bypassCSP: true,
  });
  const page = await context.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.addInitScript(() => {
    let s = 12345; // an LCG: the level meters wobble the same way every run
    Math.random = () => ((s = (Math.imul(s, 1664525) + 1013904223) >>> 0) / 2 ** 32);
  });
  // The fake clock starts at a fixed instant and only moves when `runFor` says.
  await page.clock.install({ time: new Date("2026-10-09T09:30:00Z") });
  return { context, page, errors };
}

/** A colour transition `settle` cannot see (a title fading in) would shoot half-visible text:
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
  mkdirSync(OUT, { recursive: true });
  const png = await page.screenshot({ path: resolve(OUT, `${id}-${theme}.png`) });
  const dim = { w: png.readUInt32BE(16), h: png.readUInt32BE(20) };
  expect(dim).toEqual({ w: 2800, h: 1602 });
  expect(errors).toEqual([]);
}

for (const theme of THEMES) {
  test(`desk-live ${theme}`, async ({ browser }) => {
    test.setTimeout(120_000);
    const { context, page, errors } = await shot(browser, theme);
    await page.goto("/?platform=mac#/meetings");
    await page.getByRole("button", { name: "Record call", exact: true }).click();
    await expect(page.getByRole("textbox", { name: "Meeting title" })).toBeVisible();
    await page.getByTestId("consent-hint").getByRole("button", { name: "Dismiss" }).click();
    await expect(page.getByTestId("consent-hint")).toHaveCount(0);

    const input = page.getByRole("region", { name: "Your notes" }).locator("input").last();
    for (const note of ["rename speakers live?", "plaud import", "beta scope nov"]) {
      await input.fill(note);
      await input.press("Enter");
      await expect(page.getByRole("region", { name: "Your notes" })).toContainText(note);
    }

    // One scripted line per LINE_MS; the 13th (last) one closes the sample.
    const last = page.getByText(/^Tốt\. Chốt lại: live rename/);
    for (let i = 0; i < 40 && !(await last.isVisible()); i++) {
      await page.clock.runFor(LINE_MS);
      await page.waitForTimeout(40);
    }
    await expect(last).toBeVisible();
    await expect(page.getByText(/No sound from/)).toHaveCount(0);
    await capture(page, errors, "desk-live", theme);
    await expect(page.getByText(/No sound from/)).toHaveCount(0);
    await context.close();
  });

  test(`desk-notes ${theme}`, async ({ browser }) => {
    const { context, page, errors } = await shot(browser, theme);
    await page.goto("/?platform=mac#/meetings/sample-1/notes");
    await expect(page.getByRole("heading", { name: "Summary" })).toBeVisible();
    await expect(page.getByRole("button", { name: /^Show in transcript \d+:\d\d$/ }).first()).toBeVisible();
    await expect(page.getByTestId("meeting-detail")).toBeVisible();
    await capture(page, errors, "desk-notes", theme);
    await context.close();
  });
}
