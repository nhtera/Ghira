// SPDX-License-Identifier: Apache-2.0
// UI strings (EN + VI) and the i18next setup shared by the apps and @ghi/ui.
//
// - Keys are typed: `t("live.pause")` fails to compile if the key is missing.
// - The product name is the `{{app}}` variable, never written into a string.
// - Plurals use i18next suffixes; Vietnamese has only `_other` (CLDR).
// - Dates and times: `formatDate`/`formatTime` (28/09/2026 and 24 h for vi,
//   Sep 28, 2026 for en).
import i18next, { type i18n as I18n } from "i18next";
import { initReactI18next } from "react-i18next";
import en from "../locales/en.json";
import vi from "../locales/vi.json";

export type Locale = "en" | "vi";
export type Messages = typeof en;

export const APP_NAME = "Ghira";
export const locales: Record<Locale, Messages> = { en, vi: vi as Messages };

declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: "translation";
    resources: { translation: Messages };
  }
}

/** Initializes the shared i18next instance (idempotent; switches language). */
export function initI18n(lng: Locale = "en"): I18n {
  if (!i18next.isInitialized) {
    void i18next.use(initReactI18next).init({
      lng,
      fallbackLng: "en",
      resources: { en: { translation: en }, vi: { translation: vi } },
      // React escapes text nodes; strings are never rendered as HTML (RT-6).
      interpolation: { escapeValue: false, defaultVariables: { app: APP_NAME } },
      returnNull: false,
      initAsync: false,
    });
  } else if (i18next.language !== lng) {
    void i18next.changeLanguage(lng);
  }
  return i18next;
}

export { i18next };

export { formatBytes, formatClock, formatDate, formatTime } from "./format";
