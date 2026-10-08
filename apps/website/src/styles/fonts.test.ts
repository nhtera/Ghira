// SPDX-License-Identifier: Apache-2.0

// The site serves the app's own fonts: src/styles/fonts.css has the same
// @font-face rules as packages/ui/src/fonts.css (families, subsets,
// weights, unicode ranges), only with paths into the site's node_modules.
// The preloaded files are among them.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf8");
const faces = (css: string) => [...css.matchAll(/@font-face\s*\{[^}]*\}/g)].map((m) => m[0]);

test("fonts.css has the same faces as packages/ui", () => {
  const ui = faces(read("../../../../packages/ui/src/fonts.css")).map((f) => f.replace("url(../node_modules/", "url(<modules>/"));
  const site = faces(read("./fonts.css")).map((f) => f.replace("url(../../node_modules/", "url(<modules>/"));
  assert.ok(ui.length > 0);
  assert.deepEqual(site, ui);
});

test("preloaded files are faces the CSS uses", () => {
  const css = read("./fonts.css");
  for (const m of read("./fonts.ts").matchAll(/node_modules\/(@fontsource[^?"]+)\?url/g)) {
    assert.ok(css.includes(`node_modules/${m[1]}`), m[1]);
  }
});
