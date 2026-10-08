// SPDX-License-Identifier: Apache-2.0

import { useSyncExternalStore } from "react";
import { strings } from "@/content/strings";
import { currentTheme, setTheme, subscribeTheme, type Theme } from "@/lib/theme";
import { Icon } from "./icons";

/**
 * Switches between light and dark and remembers the pick. Both icons are in
 * the markup; CSS shows the one for the current data-theme, so the button is
 * right before hydration. The label follows the theme once hydrated (the
 * server cannot know it, so it renders the light-theme label).
 */
export function ThemeToggle() {
  const theme = useSyncExternalStore<Theme>(subscribeTheme, currentTheme, () => "light");
  return (
    <button
      className="icon-btn"
      type="button"
      aria-label={theme === "dark" ? strings.theme.toLight : strings.theme.toDark}
      onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
    >
      <Icon name="moon" className="theme-moon" />
      <Icon name="sun" className="theme-sun" />
    </button>
  );
}
