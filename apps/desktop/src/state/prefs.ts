// SPDX-License-Identifier: Apache-2.0
// UI preferences of this window (theme, language, topic rail width and collapse). Kept in localStorage as a
// per-device convenience; app settings that matter to the core live in Rust.
import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import type { Locale } from "@ghi/i18n";
import type { ThemePreference } from "@ghi/ui";

export type Prefs = {
  theme: ThemePreference;
  language: Locale;
  /** Width of the transcript's Topics rail in px, set by dragging its edge. */
  topicRailWidth: number;
  /** The Topics rail folded to a slim strip with only its expand button. */
  topicRailCollapsed: boolean;
  setTheme: (t: ThemePreference) => void;
  setLanguage: (l: Locale) => void;
  setTopicRailWidth: (px: number) => void;
  setTopicRailCollapsed: (collapsed: boolean) => void;
};

export const TOPIC_RAIL_WIDTH = { min: 160, max: 420, default: 208 } as const;
export const clampRailWidth = (px: number) => Math.round(Math.min(TOPIC_RAIL_WIDTH.max, Math.max(TOPIC_RAIL_WIDTH.min, px)));

/** The system language when it is Vietnamese, else English. */
export const defaultLanguage = (): Locale =>
  typeof navigator !== "undefined" && navigator.language?.toLowerCase().startsWith("vi") ? "vi" : "en";

export const usePrefs = create<Prefs>()(
  persist(
    (set) => ({
      theme: "system",
      language: defaultLanguage(),
      topicRailWidth: TOPIC_RAIL_WIDTH.default,
      topicRailCollapsed: false,
      setTheme: (theme) => set({ theme }),
      setLanguage: (language) => set({ language }),
      setTopicRailWidth: (px) => set({ topicRailWidth: clampRailWidth(px) }),
      setTopicRailCollapsed: (topicRailCollapsed) => set({ topicRailCollapsed }),
    }),
    {
      name: "ghira.prefs",
      // Private windows and blocked storage: fall back to defaults.
      storage: createJSONStorage(() => {
        try {
          return window.localStorage;
        } catch {
          return sessionStorageFallback;
        }
      }),
      partialize: ({ theme, language, topicRailWidth, topicRailCollapsed }) => ({ theme, language, topicRailWidth, topicRailCollapsed }),
      // Whatever is stored, only known values come back.
      merge: (stored, current) => {
        const s = (stored ?? {}) as Partial<Prefs>;
        return {
          ...current,
          theme: s.theme === "light" || s.theme === "dark" || s.theme === "system" ? s.theme : current.theme,
          language: s.language === "en" || s.language === "vi" ? s.language : current.language,
          topicRailWidth: typeof s.topicRailWidth === "number" && Number.isFinite(s.topicRailWidth) ? clampRailWidth(s.topicRailWidth) : current.topicRailWidth,
          topicRailCollapsed: typeof s.topicRailCollapsed === "boolean" ? s.topicRailCollapsed : current.topicRailCollapsed,
        };
      },
    },
  ),
);

const memory = new Map<string, string>();
const sessionStorageFallback = {
  getItem: (k: string) => memory.get(k) ?? null,
  setItem: (k: string, v: string) => void memory.set(k, v),
  removeItem: (k: string) => void memory.delete(k),
};
