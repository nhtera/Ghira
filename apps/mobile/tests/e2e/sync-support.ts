// SPDX-License-Identifier: Apache-2.0
// Shared by the sync-* specs (15-I): opening the app on the scripted mock in
// one of its sync states (`?sync=<state>`, see src/ipc/mock-sync.ts).
import { expect, type Page } from "@playwright/test";

type Opts = { sync?: string; lang?: "en" | "vi"; scale?: number; dark?: boolean };

export async function openSync(page: Page, route: string, opts: Opts = {}) {
  const q = new URLSearchParams();
  if (opts.lang) q.set("lang", opts.lang);
  if (opts.scale) q.set("scale", String(opts.scale));
  if (opts.sync) q.set("sync", opts.sync);
  await page.emulateMedia({ colorScheme: opts.dark ? "dark" : "light", reducedMotion: "reduce" });
  await page.goto(`/${q.size ? `?${q}` : ""}#${route}`);
  await page.waitForFunction(() => Boolean(window.__ghiMock));
}

/** The onboarding with pairing available, the steps in `completed` done; `before` runs first (knobs). */
export async function openPairStep(page: Page, opts: Opts & { completed?: string[]; before?: () => Promise<void> } = {}) {
  await openSync(page, "/", { sync: "pairing", ...opts });
  await page.evaluate((completed) => {
    window.__ghiRecord!.setOnboarding(completed as never);
  }, opts.completed ?? ["languages", "micPriming", "consent"]);
  await opts.before?.();
  await page.evaluate(() => (window.location.hash = "#/onboarding"));
  await expect(page.locator("[data-screen=onboarding]")).toBeVisible();
}

export const mockHook = <K extends keyof NonNullable<Window["__ghiMock"]>>(page: Page, name: K, ...args: unknown[]) =>
  page.evaluate(([n, a]) => (window.__ghiMock as never as Record<string, (...x: unknown[]) => void>)[n as string](...(a as unknown[])), [name, args] as const);
