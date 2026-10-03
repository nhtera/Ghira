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

/**
 * Extra keys an app types on top of the shared ones. The iOS app augments this
 * (`apps/mobile/src/i18n-types.d.ts`) with its `mobile.*` keys, so they exist
 * only in its type surface; the desktop never sees them.
 */
// eslint-disable-next-line @typescript-eslint/no-empty-object-type
export interface ExtraMessages {}

export const APP_NAME = "Ghira";
export const locales: Record<Locale, Messages> = { en, vi: vi as Messages };

declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: "translation";
    resources: { translation: Messages & ExtraMessages };
  }
}

/** Initializes the shared instance with these resources (the iOS entry adds `mobile.*`). */
export function initResources(lng: Locale, resources: Record<Locale, object>): I18n {
  if (!i18next.isInitialized) {
    void i18next.use(initReactI18next).init({
      lng,
      fallbackLng: "en",
      resources: { en: { translation: resources.en }, vi: { translation: resources.vi } },
      // React escapes text nodes; strings are never rendered as HTML (RT-6).
      interpolation: { escapeValue: false, defaultVariables: { app: APP_NAME } },
      returnNull: false,
      initAsync: false,
    });
  } else {
    for (const l of ["en", "vi"] as const) {
      i18next.addResourceBundle(l, "translation", resources[l], true, true);
    }
    if (i18next.language !== lng) void i18next.changeLanguage(lng);
  }
  return i18next;
}

/** Initializes the shared i18next instance (idempotent; switches language). */
export function initI18n(lng: Locale = "en"): I18n {
  return initResources(lng, { en, vi });
}

export { i18next };

export { formatBytes, formatClock, formatDate, formatTime } from "./format";
