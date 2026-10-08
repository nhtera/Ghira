// SPDX-License-Identifier: Apache-2.0

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { ghiraTheme } from "./shiki.ts";

// Code is drawn on --surface (.code). Every colour the theme uses must be
// AA (4.5:1) there, in both themes.
const css = readFileSync(new URL("../../../../packages/ui/src/tokens/tokens.css", import.meta.url), "utf8");

function block(selector: string): Map<string, string> {
  const start = css.indexOf(selector);
  assert.ok(start >= 0, selector);
  const body = css.slice(css.indexOf("{", start) + 1, css.indexOf("}", start));
  return new Map([...body.matchAll(/(--[a-z0-9-]+):\s*(#[0-9A-Fa-f]{6})/g)].map((m) => [m[1], m[2]]));
}

function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const used = new Set<string>();
for (const rule of ghiraTheme.tokenColors ?? []) {
  const fg = rule.settings?.foreground;
  const m = fg && /^var\(--([a-z0-9-]+)\)$/.exec(fg);
  assert.ok(m, `token colours must be var(--token): ${fg}`);
  used.add(`--${m[1]}`);
}
used.add("--ink");

for (const [name, selector] of [
  ["light", ':root,\n[data-theme="light"]'],
  ["dark", '[data-theme="dark"]'],
] as const) {
  test(`code colours are AA on --surface (${name})`, () => {
    const tokens = block(selector);
    const surface = tokens.get("--surface");
    assert.ok(surface);
    for (const token of used) {
      const colour = tokens.get(token);
      assert.ok(colour, `${token} is not a colour token`);
      assert.ok(contrast(colour, surface) >= 4.5, `${token} ${colour} on ${surface}: ${contrast(colour, surface).toFixed(2)}`);
    }
  });
}
