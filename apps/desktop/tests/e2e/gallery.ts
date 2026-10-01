// SPDX-License-Identifier: Apache-2.0
// The gallery's stories, read from the page (the gallery lists them from its
// glob, so a new story is covered without editing a test).
import type { Page } from "@playwright/test";

export type Entry = { id: string; title: string; states: string[] };

export async function galleryEntries(page: Page): Promise<Entry[]> {
  await page.goto("/");
  return page.evaluate(() => (window as unknown as { __GALLERY__: Entry[] }).__GALLERY__);
}

export const VARIANTS = [
  { theme: "light", lang: "en" },
  { theme: "dark", lang: "en" },
  { theme: "light", lang: "vi" },
  { theme: "dark", lang: "vi" },
] as const;

export const storyUrl = (id: string, v: { theme: string; lang: string }, extra: Record<string, string> = {}) =>
  `/?${new URLSearchParams({ story: id, ...v, ...extra })}`;
