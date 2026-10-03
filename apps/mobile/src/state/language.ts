// SPDX-License-Identifier: Apache-2.0
import type { Locale } from "@ghi/i18n";

/** The phone's language when it is Vietnamese, else English. Settings → Languages (16-J) overrides it. */
export const deviceLanguage = (): Locale =>
  typeof navigator !== "undefined" && navigator.language?.toLowerCase().startsWith("vi") ? "vi" : "en";

/**
 * Outside the app (the browser on the scripted mock), e2e pins the language
 * and text scale with `?lang=vi&scale=2`. Inside Tauri the phone decides.
 */
export function browserOverrides(): { lang?: Locale; scale?: number } {
  if (typeof window === "undefined" || "__TAURI_INTERNALS__" in window) return {};
  const q = new URLSearchParams(window.location.search);
  const lang = q.get("lang");
  const scale = Number(q.get("scale"));
  return {
    lang: lang === "vi" || lang === "en" ? lang : undefined,
    scale: scale > 0 && scale <= 3 ? scale : undefined,
  };
}
