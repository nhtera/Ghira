// SPDX-License-Identifier: Apache-2.0

// Every token the site's CSS uses must exist in packages/ui's tokens.css,
// as a plain CSS variable (not only inside Tailwind's `@theme inline`, which
// emits nothing the site can rely on). The site imports the file as it is,
// so a rename there breaks the site here first. The site defines only its
// layout variables, and copies no palette (no hex colors).

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const here = new URL(".", import.meta.url);
const read = (rel: string) => readFileSync(new URL(rel, here), "utf8");

const tokens = read("../../../../packages/ui/src/tokens/tokens.css");
// Plain variables only: drop the @theme block before collecting names.
const plain = tokens.replace(/@theme inline \{[\s\S]*?\n\}/, "");
const defined = new Set([...plain.matchAll(/(--[a-z0-9-]+)\s*:/g)].map((m) => m[1]));

/** The only variables the site defines itself (src/styles/site.css). */
export const SITE_VARIABLES = ["--wrap", "--gutter", "--show-light", "--show-dark", "--bezel"];

const FILES = ["./site.css", "./landing.css", "./docs.css", "../lib/shiki.ts"];
const used = (source: string) => [...new Set([...source.matchAll(/var\((--[a-z0-9-]+)/g)].map((m) => m[1]))];

for (const file of FILES) {
  test(`tokens used by ${file} exist in tokens.css`, () => {
    const undefinedNames = used(read(file)).filter((name) => !defined.has(name) && !SITE_VARIABLES.includes(name));
    assert.deepEqual(undefinedNames, [], `not defined as plain variables in packages/ui tokens.css: ${undefinedNames.join(", ")}`);
  });
}

test("the site defines only its layout variables", () => {
  for (const file of ["./site.css", "./landing.css", "./docs.css"]) {
    const own = [...read(file).matchAll(/^\s*(--[a-z0-9-]+)\s*:/gm)].map((m) => m[1]);
    assert.deepEqual(own.filter((n) => !SITE_VARIABLES.includes(n)), [], file);
  }
});

test("tokens.css defines both themes and the plain font and radius variables", () => {
  assert.match(tokens, /:root,\s*\[data-theme="light"\]\s*\{/);
  assert.match(tokens, /\[data-theme="dark"\]\s*\{/);
  for (const name of ["--font-sans", "--font-serif", "--font-mono", "--radius-seg", "--radius-ctl", "--radius-row", "--radius-panel", "--radius-dialog"]) {
    assert.ok(defined.has(name), name);
  }
});

test("the site copies no palette: no hex colors in its CSS", () => {
  for (const file of ["./site.css", "./landing.css", "./docs.css"]) {
    assert.deepEqual(read(file).match(/#[0-9a-fA-F]{3,8}\b/g) ?? [], [], file);
  }
});
