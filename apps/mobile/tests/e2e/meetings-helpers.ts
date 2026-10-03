// SPDX-License-Identifier: Apache-2.0
// Helpers for the meetings, meeting view and search specs (16-I).
import { expect, type Page } from "@playwright/test";

export type Opts = {
  lang?: "en" | "vi";
  scale?: number;
  meetings?: "empty" | "many";
  dark?: boolean;
};

/** Opens a route on the scripted mock (`?meetings=` picks the dataset). */
export async function openMeetings(page: Page, route: string, opts: Opts = {}) {
  const q = new URLSearchParams();
  if (opts.lang) q.set("lang", opts.lang);
  if (opts.scale) q.set("scale", String(opts.scale));
  if (opts.meetings) q.set("meetings", opts.meetings);
  if (opts.locked) q.set("locked", "1");
  if (opts.starting) q.set("starting", String(opts.starting));
  await page.emulateMedia({
    colorScheme: opts.dark ? "dark" : "light",
    reducedMotion: "reduce",
  });
  await page.goto(`/${q.size ? `?${q}` : ""}#${route}`);
  await page.waitForFunction(() => Boolean(window.__ghiMock));
}

type Recorded = {
  audio: { op: string; t: number }[];
  copied: string[];
};
declare global {
  interface Window {
    __rec: Recorded;
  }
}

/**
 * Before the page loads: records what the screens ask of the platform instead
 * of doing it (audio play / pause with the position, navigator.share, the
 * cloud-sheet hand-off). WebKit here has no audio file to play.
 */
export async function recordPlatform(page: Page) {
  await page.addInitScript(() => {
    const rec: Recorded = { audio: [], copied: [] };
    window.__rec = rec;
    const times = new WeakMap<object, number>();
    Object.defineProperty(HTMLMediaElement.prototype, "currentTime", {
      configurable: true,
      get() {
        return times.get(this) ?? 0;
      },
      set(v: number) {
        times.set(this, v);
      },
    });
    // No file behind the token: do not try to load it (an error would mark the audio unavailable).
    Object.defineProperty(HTMLMediaElement.prototype, "src", {
      configurable: true,
      get: () => "",
      set: () => undefined,
    });
    const playing = new WeakSet<object>();
    Object.defineProperty(HTMLMediaElement.prototype, "paused", {
      configurable: true,
      get() {
        return !playing.has(this);
      },
    });
    HTMLMediaElement.prototype.play = function (this: HTMLMediaElement) {
      playing.add(this);
      rec.audio.push({ op: "play", t: this.currentTime });
      this.dispatchEvent(new Event("play"));
      return Promise.resolve();
    };
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: async (text: string) => void rec.copied.push(text) },
    });
    HTMLMediaElement.prototype.pause = function (this: HTMLMediaElement) {
      playing.delete(this);
      rec.audio.push({ op: "pause", t: this.currentTime });
      this.dispatchEvent(new Event("pause"));
    };
  });
}

export const recorded = (page: Page) => page.evaluate(() => window.__rec);

export async function expectRows(page: Page, ids: string[]) {
  for (const id of ids)
    await expect(page.locator(`[data-meeting="${id}"]`)).toBeVisible();
}

type Hooks = {
  failDeletes(v: boolean): void;
  failSaves(v: boolean): void;
  failShare(v: boolean): void;
  failAudio(v: boolean): void;
};
/** Flips the mock core's failure switches (see mock-meetings.ts). */
export const mock = <K extends keyof Hooks>(
  page: Page,
  hook: K,
  value: boolean,
) =>
  page.evaluate(
    ([h, v]) =>
      (
        window as unknown as {
          __ghiMeetings: Record<string, (v: boolean) => void>;
        }
      ).__ghiMeetings[h](v),
    [hook, value] as const,
  );
