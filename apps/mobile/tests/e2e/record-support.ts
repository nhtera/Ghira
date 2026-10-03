// SPDX-License-Identifier: Apache-2.0
// Shared by the onboarding-* and record-* specs (16-H): scripted-mock knobs
// (`window.__ghiRecord`) and the screens' common steps.
import { expect, type Page } from "@playwright/test";
import { openApp } from "./helpers";

type Opts = { lang?: "en" | "vi"; scale?: number };
type Knobs = Record<string, unknown>;

/** Sets `window.__ghiRecord` fields (see src/ipc/mock-record.ts). */
export async function setKnobs(page: Page, knobs: Knobs) {
  await page.evaluate((k) => Object.assign(window.__ghiRecord as object, k), knobs);
}

/** Steps already completed before the onboarding opens ([] = first launch). */
export async function openOnboarding(page: Page, opts: Opts & { completed?: string[]; knobs?: Knobs } = {}) {
  await openApp(page, "/", opts);
  await page.evaluate(({ completed, knobs }) => {
    Object.assign(window.__ghiRecord as object, knobs);
    window.__ghiRecord!.setOnboarding(completed as never);
    window.location.hash = "#/onboarding";
  }, { completed: opts.completed ?? [], knobs: opts.knobs ?? {} });
  await expect(page.locator("[data-screen=onboarding]")).toBeVisible();
}

/** The Record tab, onboarding done. */
export async function openRecord(page: Page, opts: Opts & { knobs?: Knobs } = {}) {
  await openApp(page, "/", opts);
  await page.evaluate((knobs) => {
    Object.assign(window.__ghiRecord as object, knobs);
    window.location.hash = "#/record";
  }, opts.knobs ?? {});
  await expect(page.locator("[data-screen=record]")).toBeVisible();
}

/** Records the clipboard writes (WebKit in tests has none to read). */
export async function stubClipboard(page: Page) {
  await page.addInitScript(() => {
    const w = window as unknown as { __clip: string[] };
    w.__clip = [];
    Object.defineProperty(navigator, "clipboard", { value: { writeText: async (t: string) => void w.__clip.push(t) }, configurable: true });
  });
}

export const clipboardWrites = (page: Page) => page.evaluate(() => (window as unknown as { __clip: string[] }).__clip);

export const log = (page: Page) => page.evaluate(() => window.__ghiRecord!.log);

/** Start a recording through the UI (consent sheet and all). */
export async function startRecording(page: Page, name: RegExp | string = /record room|ghi phòng họp/i) {
  await page.getByRole("button", { name }).click();
  await page.getByRole("button", { name: /start recording|bắt đầu ghi âm/i }).click();
}
