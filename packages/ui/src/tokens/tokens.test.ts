// SPDX-License-Identifier: Apache-2.0
// @vitest-environment node
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
// @ts-expect-error: a plain .mjs build script, no types
import { tokensCss } from "../../scripts/build-tokens-css.mjs";
import { CVD, contrast, deltaE, simulate } from "./color-math";
import tokens from "./tokens.json";
import { colors, speakerSlot, type ColorToken, type Theme } from "./tokens";

const themes: Theme[] = ["light", "dark"];

describe("tokens", () => {
  it("tokens.css is generated from tokens.json (pnpm --filter @ghi/ui gen:tokens)", () => {
    const css = readFileSync(new URL("./tokens.css", import.meta.url), "utf8");
    expect(css).toBe(tokensCss(tokens));
  });

  it("both themes define the same tokens", () => {
    expect(Object.keys(colors.dark)).toEqual(Object.keys(colors.light));
  });

  it("speaker slots follow the CVD-safe order and cycle", () => {
    expect([0, 1, 2, 3, 8].map(speakerSlot)).toEqual(["s1", "s2", "s4", "s8", "s1"]);
  });
});

describe.each(themes)("%s theme contrast (WCAG 2.2 AA)", (theme) => {
  const c = colors[theme];
  it.each(["ink", "muted", "faint", "accent", "recInk", "warn", "ai"] as const)("%s text on surface ≥ 4.5:1", (k) => {
    expect(contrast(c[k], c.surface)).toBeGreaterThanOrEqual(4.5);
  });
  // faint is for text on surface only (tokens.json); muted is the hint color elsewhere.
  it.each(["ink", "muted", "accent", "recInk"] as const)("%s text on every background ≥ 4.5:1", (k) => {
    for (const bg of [c.bg, c.surface2, c.sunk]) expect(contrast(c[k], bg)).toBeGreaterThanOrEqual(4.5);
  });
  it.each(["ctl", "accent", "rec"] as const)("%s strokes ≥ 3:1 on every background", (k) => {
    for (const bg of [c.bg, c.surface, c.surface2, c.sunk]) expect(contrast(c[k], bg)).toBeGreaterThanOrEqual(3);
  });
  it.each([
    ["onAccent", "accent"],
    ["accent", "accentSoft"],
    ["recInk", "recSoft"],
    ["warn", "warnSoft"],
    ["toastFg", "toastBg"],
  ] as const)("%s on %s ≥ 4.5:1", (fg, bg) => {
    expect(contrast(c[fg], c[bg])).toBeGreaterThanOrEqual(4.5);
  });
  it("speaker initials stay readable on every slot (≥ 4.5:1: small text)", () => {
    for (const s of ["s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8"] as const) {
      expect(contrast(c.onS, c[s]), s).toBeGreaterThanOrEqual(4.5);
    }
  });
});

describe.each(themes)("%s theme speaker palette under color-vision deficiencies", (theme) => {
  const first4 = (["s1", "s2", "s4", "s8"] as const).map((s) => colors[theme][s]);
  it.each([["normal", null], ...Object.entries(CVD)] as const)("%s: first four slots ΔE ≥ 0.049", (_, m) => {
    const seen = first4.map((h) => (m ? simulate(h, m) : h));
    for (let a = 0; a < seen.length; a++)
      for (let b = a + 1; b < seen.length; b++) expect(deltaE(seen[a], seen[b])).toBeGreaterThanOrEqual(0.049);
  });
});

describe("iOS speaker palette (tokens.json iosOverride, ios.css)", () => {
  const ios: Record<Theme, Record<ColorToken, string>> = {
    light: { ...colors.light, ...tokens.iosOverride.light },
    dark: { ...colors.dark },
  };
  const css = readFileSync(new URL("./ios.css", import.meta.url), "utf8");

  it("ios.css carries the override", () => {
    for (const [k, v] of Object.entries(tokens.iosOverride.light)) expect(css.toLowerCase()).toContain(`--${k}: ${v.toLowerCase()};`);
  });

  describe.each(themes)("%s", (theme) => {
    const c = ios[theme];
    const slots = ["s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8"] as const;
    it.each(slots)("%s text on bg, surface and surface2 >= 4.5:1", (s) => {
      for (const bg of [c.bg, c.surface, c.surface2]) expect(contrast(c[s], bg)).toBeGreaterThanOrEqual(4.5);
    });
    it.each(slots)("initials (onS) on %s >= 4.5:1", (s) => {
      expect(contrast(c.onS, c[s])).toBeGreaterThanOrEqual(4.5);
    });
    it.each([["normal", null], ...Object.entries(CVD)] as const)("%s: first four slots stay apart (dE >= 0.049)", (_, m) => {
      const seen = (["s1", "s2", "s4", "s8"] as const).map((s) => (m ? simulate(c[s], m) : c[s]));
      for (let a = 0; a < seen.length; a++) for (let b = a + 1; b < seen.length; b++) expect(deltaE(seen[a], seen[b])).toBeGreaterThanOrEqual(0.049);
    });
  });
});
