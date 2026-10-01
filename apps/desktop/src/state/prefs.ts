// SPDX-License-Identifier: Apache-2.0
// UI preferences of this window (theme, language). Kept in localStorage as a
// per-device convenience; app settings that matter to the core live in Rust.
import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import type { Locale } from "@ghi/i18n";
import type { ThemePreference } from "@ghi/ui";

export type Prefs = {
  theme: ThemePreference;
  language: Locale;
  setTheme: (t: ThemePreference) => void;
  setLanguage: (l: Locale) => void;
};

/** The system language when it is Vietnamese, else English. */
export const defaultLanguage = (): Locale =>
  typeof navigator !== "undefined" && navigator.language?.toLowerCase().startsWith("vi") ? "vi" : "en";

export const usePrefs = create<Prefs>()(
  persist(
    (set) => ({
      theme: "system",
      language: defaultLanguage(),
      setTheme: (theme) => set({ theme }),
      setLanguage: (language) => set({ language }),
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
      partialize: ({ theme, language }) => ({ theme, language }),
      // Whatever is stored, only known values come back.
      merge: (stored, current) => {
        const s = (stored ?? {}) as Partial<Prefs>;
        return {
          ...current,
          theme: s.theme === "light" || s.theme === "dark" || s.theme === "system" ? s.theme : current.theme,
          language: s.language === "en" || s.language === "vi" ? s.language : current.language,
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
