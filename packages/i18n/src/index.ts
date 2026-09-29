// SPDX-License-Identifier: Apache-2.0
// UI strings. i18next is wired up in phase 9; until then the app reads these directly.
import en from "../locales/en.json";
import vi from "../locales/vi.json";

export type Locale = "en" | "vi";
export type Messages = typeof en;

export const locales: Record<Locale, Messages> = { en, vi };
