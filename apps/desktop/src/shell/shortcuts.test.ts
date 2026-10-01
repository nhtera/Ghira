// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { matchChord, shortcutFor } from "./shortcuts";

const key = (k: string, mods: Partial<Record<"metaKey" | "ctrlKey" | "shiftKey" | "altKey", boolean>> = {}) =>
  ({ key: k, metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...mods }) as KeyboardEvent;

describe("shortcuts", () => {
  it("Mod is ⌘ on mac and Ctrl on Windows", () => {
    expect(matchChord(key("r", { metaKey: true, shiftKey: true }), "Mod+Shift+R", "mac")).toBe(true);
    expect(matchChord(key("R", { ctrlKey: true, shiftKey: true }), "Mod+Shift+R", "win")).toBe(true);
    expect(matchChord(key("r", { ctrlKey: true, shiftKey: true }), "Mod+Shift+R", "mac")).toBe(false);
    expect(matchChord(key("m", { metaKey: true, altKey: true }), "Mod+M", "mac")).toBe(false);
  });

  it("with the macOS menu, the menu owns ⌘M and ⌘K; the webview handles ⌘F", () => {
    expect(shortcutFor(key("m", { metaKey: true }), "mac", true)).toBe(null);
    expect(shortcutFor(key("k", { metaKey: true }), "mac", true)).toBe(null);
    expect(shortcutFor(key("f", { metaKey: true }), "mac", true)).toBe("find");
    expect(shortcutFor(key("k", { metaKey: true }), "mac", false)).toBe("commandPalette");
    expect(shortcutFor(key("m", { ctrlKey: true }), "win", false)).toBe("mark");
    expect(shortcutFor(key("k", { ctrlKey: true }), "win", false)).toBe("commandPalette");
  });
});
