// SPDX-License-Identifier: Apache-2.0
// Every locale must define exactly the same keys as English.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const load = (name) => JSON.parse(readFileSync(new URL(`../locales/${name}.json`, import.meta.url), "utf8"));
const keys = (obj, prefix = "") =>
  Object.entries(obj).flatMap(([k, v]) =>
    v && typeof v === "object" ? keys(v, `${prefix}${k}.`) : [`${prefix}${k}`],
  ).sort();

test("vi has the same keys as en", () => {
  assert.deepEqual(keys(load("vi")), keys(load("en")));
});
