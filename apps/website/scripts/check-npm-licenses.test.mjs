// SPDX-License-Identifier: Apache-2.0

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { ALLOWED, checkLock, isAllowed } from "./check-npm-licenses.mjs";

test("the allowed list equals the repository's (tools/scripts/check-js-licenses.mjs)", () => {
  const repo = readFileSync(new URL("../../../tools/scripts/check-js-licenses.mjs", import.meta.url), "utf8");
  const block = repo.match(/const allowed = \[([\s\S]*?)\];/)?.[1];
  assert.ok(block, "allowed list not found in check-js-licenses.mjs");
  const ids = [...block.replace(/\/\/.*$/gm, "").matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  assert.deepEqual([...ids].sort(), [...ALLOWED].sort());
});

test("SPDX expressions", () => {
  assert.ok(isAllowed("MIT"));
  assert.ok(isAllowed("(MIT OR GPL-3.0)"));
  assert.ok(isAllowed("MIT AND Apache-2.0"));
  assert.ok(!isAllowed("MIT AND GPL-3.0"));
  assert.ok(!isAllowed("LGPL-3.0-or-later"));
  assert.ok(!isAllowed("SEE LICENSE IN LICENSE"));
  assert.ok(!isAllowed(undefined));
  assert.ok(!isAllowed("(MIT"));
});

test("dev packages and build-only tools are skipped; a shipped GPL package fails", () => {
  const { bad, n } = checkLock({
    packages: {
      "": {},
      "node_modules/a": { license: "MIT" },
      "node_modules/b": { license: "GPL-3.0", dev: true },
      "node_modules/caniuse-lite": { license: "CC-BY-4.0" },
      "node_modules/c": { license: "GPL-3.0" },
    },
  });
  assert.equal(n, 2);
  assert.deepEqual(bad, ["c: license GPL-3.0 is not allowed"]);
});
