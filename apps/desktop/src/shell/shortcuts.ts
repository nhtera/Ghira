// SPDX-License-Identifier: Apache-2.0
// The keyboard map (brief §8). "Mod" is ⌘ on mac and Ctrl on Windows. On mac
// the app menu owns the chords that native menus would catch (⌘⇧R, ⌘M, ⌘K,
// ⌘,) and sends them as `menuAction`; the webview handles the rest, and all of
// them on Windows.
import type { Platform } from "@ghi/ui";

export type ShortcutId =
  | "toggleRecording"
  | "mark"
  | "commandPalette"
  | "find"
  | "notesTab"
  | "transcriptTab"
  | "export"
  | "settings";

export const SHORTCUTS: Record<ShortcutId, string> = {
  toggleRecording: "Mod+Shift+R",
  mark: "Mod+M",
  commandPalette: "Mod+K",
  find: "Mod+F",
  notesTab: "Mod+1",
  transcriptTab: "Mod+2",
  export: "Mod+E",
  settings: "Mod+,",
};

/** Chords the macOS menu handles (they never reach the webview there). */
export const MENU_OWNED: ShortcutId[] = ["toggleRecording", "mark", "commandPalette", "settings"];

export function matchChord(e: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">, chord: string, platform: Platform): boolean {
  const parts = chord.split("+");
  const key = parts.pop()!.toLowerCase();
  const want = new Set(parts);
  const mod = platform === "mac" ? e.metaKey : e.ctrlKey;
  const other = platform === "mac" ? e.ctrlKey : e.metaKey;
  return (
    e.key.toLowerCase() === key &&
    mod === want.has("Mod") &&
    e.shiftKey === want.has("Shift") &&
    e.altKey === want.has("Alt") &&
    !other
  );
}

/**
 * The shortcut a key event triggers, if any. `nativeMenu`: the macOS app menu
 * is present (inside Tauri) and already handles MENU_OWNED.
 */
export function shortcutFor(e: KeyboardEvent, platform: Platform, nativeMenu: boolean): ShortcutId | null {
  for (const [id, chord] of Object.entries(SHORTCUTS) as [ShortcutId, string][]) {
    if (nativeMenu && platform === "mac" && MENU_OWNED.includes(id)) continue;
    if (matchChord(e, chord, platform)) return id;
  }
  return null;
}
