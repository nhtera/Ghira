// SPDX-License-Identifier: Apache-2.0
// Light / dark / system. The theme lives on <html data-theme>; "system"
// follows prefers-color-scheme and updates when the OS switches.
import { useEffect, useSyncExternalStore } from "react";

export type ThemePreference = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

const query = () =>
  typeof window !== "undefined" && window.matchMedia ? window.matchMedia("(prefers-color-scheme: dark)") : null;

export function resolveTheme(pref: ThemePreference): ResolvedTheme {
  if (pref !== "system") return pref;
  return query()?.matches ? "dark" : "light";
}

/** Sets <html data-theme>; call before the first render to avoid a flash. */
export function applyTheme(pref: ThemePreference, root: HTMLElement = document.documentElement): ResolvedTheme {
  const t = resolveTheme(pref);
  root.dataset.theme = t;
  return t;
}

function subscribe(onChange: () => void) {
  const mq = query();
  mq?.addEventListener("change", onChange);
  return () => mq?.removeEventListener("change", onChange);
}

/** Applies `pref` and keeps following the OS while it is "system". */
export function useTheme(pref: ThemePreference): ResolvedTheme {
  const systemDark = useSyncExternalStore(subscribe, () => query()?.matches ?? false, () => false);
  const theme: ResolvedTheme = pref === "system" ? (systemDark ? "dark" : "light") : pref;
  useEffect(() => {
    applyTheme(theme);
  }, [theme]);
  return theme;
}
