// SPDX-License-Identifier: Apache-2.0
import { expect, test } from "vitest";
import { build, format, varsOf } from "../scripts/ios-strings-lib.mjs";

test("one variable stays plain, several are positional in EN order", () => {
  expect(format("Recording {{count}}", ["count"])).toBe("Recording %lld");
  expect(format("{{name}} at {{time}}", ["name", "time"])).toBe("%1$@ at %2$@");
  // VI reorders the words; the positions follow EN's variable order.
  expect(format("lúc {{time}} {{name}}", ["name", "time"])).toBe("lúc %2$@ %1$@");
  expect(format("{{count}} marks on {{device}}", ["count", "device"])).toBe("%1$lld marks on %2$@");
});

test("{{app}} is the product name and a literal % is escaped", () => {
  expect(format("{{app}} uses 100% on-device", [])).toBe("Ghira uses 100%% on-device");
  expect(varsOf("{{app}} {{a}} {{b}} {{a}}")).toEqual(["a", "b"]);
});

test("plural keys become one entry with variations and the same positions", () => {
  const en = {
    "mobile.ios.x.marks_one": "{{count}} mark on {{device}}",
    "mobile.ios.x.marks_other": "{{count}} marks on {{device}}",
  };
  const vi = { "mobile.ios.x.marks_other": "{{count}} dấu trên {{device}}" };
  const json = JSON.parse(build({ en, vi }));
  const loc = json.strings["mobile.ios.x.marks"].localizations;
  expect(loc.en.variations.plural.one.stringUnit.value).toBe("%1$lld mark on %2$@");
  expect(loc.vi.variations.plural.other.stringUnit.value).toBe("%1$lld dấu trên %2$@");
});
