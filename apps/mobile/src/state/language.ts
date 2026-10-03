// SPDX-License-Identifier: Apache-2.0
import type { Locale } from "@ghi/i18n";

/** The phone's language when it is Vietnamese, else English. Settings → Languages (16-J) overrides it. */
export const deviceLanguage = (): Locale =>
  typeof navigator !== "undefined" && navigator.language?.toLowerCase().startsWith("vi") ? "vi" : "en";
