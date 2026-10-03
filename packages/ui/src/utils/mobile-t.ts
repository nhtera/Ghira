// SPDX-License-Identifier: Apache-2.0
// `mobile.*` keys exist only in the phone app's type surface (the desktop never
// sees them), so @ghi/ui types them from the locale files directly instead of
// through i18next's global key type. The iOS variants only run where the app
// (or the gallery with platform=ios) loaded the mobile strings.
import type { MobileOnly } from "@ghi/i18n/mobile";
import { useTranslation } from "react-i18next";

type Paths<T> = {
  [K in keyof T & string]: T[K] extends string ? K : `${K}.${Paths<T[K]>}`;
}[keyof T & string];

/** Every `mobile.*` leaf key, including i18next plural suffixes as written in the files. */
export type MobileKey = Paths<MobileOnly>;

export type MobileT = (key: MobileKey, options?: Record<string, unknown>) => string;

export function useMobileT(): MobileT {
  return useTranslation().t as unknown as MobileT;
}
