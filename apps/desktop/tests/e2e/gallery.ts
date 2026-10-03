// SPDX-License-Identifier: Apache-2.0
// The gallery's stories, read from the page (the gallery lists them from its
// glob, so a new story is covered without editing a test).
import type { Page } from "@playwright/test";

/** `platform: "ios"` stories are the phone's: visit them with `platform=ios`, never as mac/win. */
export type Entry = { id: string; title: string; states: string[]; platform?: "ios"; overlays: string[] };

export async function galleryEntries(page: Page): Promise<Entry[]> {
  await page.goto("/");
  return page.evaluate(() => (window as unknown as { __GALLERY__: Entry[] }).__GALLERY__);
}

export const isPhone = (e: Entry) => e.platform === "ios";

export const VARIANTS = [
  { theme: "light", lang: "en" },
  { theme: "dark", lang: "en" },
  { theme: "light", lang: "vi" },
  { theme: "dark", lang: "vi" },
] as const;

export const storyUrl = (id: string, v: { theme: string; lang: string }, extra: Record<string, string> = {}) =>
  `/?${new URLSearchParams({ story: id, ...v, ...extra })}`;

/** The phone's matrix: the four theme/language pairs, plus 200% text in Vietnamese (stacked diacritics). */
export const PHONE_VARIANTS = [
  ...VARIANTS.map((v) => ({ ...v, scale: "1" })),
  { theme: "light", lang: "vi", scale: "2" },
] as const;

export const phoneUrl = (id: string, v: { theme: string; lang: string; scale: string }, extra: Record<string, string> = {}) =>
  storyUrl(id, v, { platform: "ios", scale: v.scale, ...extra });
