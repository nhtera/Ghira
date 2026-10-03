// SPDX-License-Identifier: Apache-2.0
// mac vs Windows (desktop) and iOS (the phone app): icons, dialog shapes,
// shortcut labels and platform copy (`t(key, { context: usePlatform() })`).
// Detected once; tests and the gallery override it with <PlatformProvider value>.
//
// `Platform` is the desktop pair, so desktop code that builds `_mac` / `_win`
// keys stays exact. The phone app is `AppPlatform` "ios": `usePlatform()` reads
// it as "mac" (Material icons, `_mac` copy) and `useAppPlatform()` / 
// `usePlatformContext()` see "ios" and pick a key's `_ios` variant when it has one.
import { createContext, useContext, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

export type Platform = "mac" | "win";
export type AppPlatform = Platform | "ios";

/** The desktop platform an app platform stands in for: iOS reads the `_mac` variants. */
export function copyContext(platform: AppPlatform): Platform {
  return platform === "win" ? "win" : "mac";
}

export function detectPlatform(): Platform {
  if (typeof navigator === "undefined") return "mac";
  return /Windows/i.test(navigator.userAgent) ? "win" : "mac";
}

/** The platform including iOS (an iPhone user agent). The phone app provides "ios" itself. */
export function detectAppPlatform(): AppPlatform {
  if (typeof navigator !== "undefined" && /iPhone|iPod/i.test(navigator.userAgent)) return "ios";
  return detectPlatform();
}

const PlatformContext = createContext<AppPlatform>(detectPlatform());

export function PlatformProvider({ value, children }: { value: AppPlatform; children: ReactNode }) {
  return <PlatformContext.Provider value={value}>{children}</PlatformContext.Provider>;
}

/** The desktop platform (iOS reads as "mac"). */
export function usePlatform(): Platform {
  return copyContext(useContext(PlatformContext));
}

/** The app platform, "ios" included. */
export function useAppPlatform(): AppPlatform {
  return useContext(PlatformContext);
}

/**
 * The i18n context for a platform-copy key: iOS reads `key_ios` when the key
 * has one and falls back to `key_mac`; mac and Windows read their own variant.
 */
export function usePlatformContext(): (key: string) => Platform {
  const platform = useAppPlatform();
  const { i18n } = useTranslation();
  // Typed keys declare only `_mac` / `_win` variants today; an `_ios` variant
  // is picked at runtime, so the value is typed as the variants i18next knows.
  return (key) =>
    (platform === "ios" && i18n.exists(key, { context: "ios" }) ? "ios" : copyContext(platform)) as Platform;
}

/** "⌘K" on mac, "Ctrl+K" on Windows, from a "Mod+K" style chord. */
export function shortcutLabel(chord: string, platform: Platform): string {
  const parts = chord.split("+");
  if (platform === "mac") {
    const sym: Record<string, string> = { Mod: "⌘", Shift: "⇧", Alt: "⌥", Ctrl: "⌃" };
    return parts.map((p) => sym[p] ?? p).join("");
  }
  return parts.map((p) => (p === "Mod" ? "Ctrl" : p)).join("+");
}
