// SPDX-License-Identifier: Apache-2.0
// mac vs Windows: icons, dialog shapes, shortcut labels and platform copy
// (`t(key, { context: platform })`). Detected once; tests and the gallery
// override it with <PlatformProvider value>.
import { createContext, useContext, type ReactNode } from "react";

export type Platform = "mac" | "win";

export function detectPlatform(): Platform {
  if (typeof navigator === "undefined") return "mac";
  return /Windows/i.test(navigator.userAgent) ? "win" : "mac";
}

const PlatformContext = createContext<Platform>(detectPlatform());

export function PlatformProvider({ value, children }: { value: Platform; children: ReactNode }) {
  return <PlatformContext.Provider value={value}>{children}</PlatformContext.Provider>;
}

export function usePlatform(): Platform {
  return useContext(PlatformContext);
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
