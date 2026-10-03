// SPDX-License-Identifier: Apache-2.0
// The iOS app's entry (`@ghi/i18n/mobile`): the desktop strings plus
// `mobile.*`. Kept apart from the main entry so the desktop bundle and type
// surface never carry the mobile strings.
import type { i18n as I18n } from "i18next";
import en from "../locales/en.json";
import vi from "../locales/vi.json";
import { initResources, type Locale } from "./index";
import { mobileLocales } from "./mobile";

export { MOBILE_FILES, mobileLocales, type MobileOnly } from "./mobile";

/** The iOS app's instance: the desktop strings plus `mobile.*`. */
export function initMobileI18n(lng: Locale = "en"): I18n {
  return initResources(lng, {
    en: { ...en, ...mobileLocales.en },
    vi: { ...vi, ...mobileLocales.vi },
  });
}
