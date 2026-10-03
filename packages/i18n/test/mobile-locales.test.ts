// SPDX-License-Identifier: Apache-2.0
// The mobile strings (locales/mobile/*) follow the same rules as the desktop
// ones, live only under `mobile.*`, and never redefine a desktop key.
import fs from "node:fs";
import path from "node:path";
import { expect, test } from "vitest";
import en from "../locales/en.json";
import vi from "../locales/vi.json";
import { initMobileI18n, MOBILE_FILES, mobileLocales } from "../src/mobile-entry";

type Tree = { [key: string]: string | Tree };

function flatten(tree: Tree, prefix = ""): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(tree)) {
    if (typeof v === "string") out[`${prefix}${k}`] = v;
    else Object.assign(out, flatten(v, `${prefix}${k}.`));
  }
  return out;
}

const PLURAL = /_(zero|one|two|few|many|other)$/;
const base = (key: string) => key.replace(PLURAL, "");
const vars = (s: string) =>
  [...new Set([...s.matchAll(/\{\{(\w+)\}\}/g)].map((m) => m[1]))].filter((v) => v !== "app").sort();
const forms = (flat: Record<string, string>) => {
  const m = new Map<string, string[]>();
  for (const key of Object.keys(flat)) {
    if (PLURAL.test(key)) m.set(base(key), [...(m.get(base(key)) ?? []), key.match(PLURAL)![1]]);
  }
  return m;
};

const E = flatten(mobileLocales.en as unknown as Tree);
const V = flatten(mobileLocales.vi as unknown as Tree);
const dir = path.join(__dirname, "../locales/mobile");

test("every file in locales/mobile is loaded, in both languages", () => {
  const stems = fs
    .readdirSync(dir)
    .filter((f) => f.endsWith(".json"))
    .map((f) => f.replace(/\.(en|vi)\.json$/, ""));
  expect([...new Set(stems)].sort()).toEqual([...MOBILE_FILES].sort());
  for (const stem of MOBILE_FILES) {
    for (const lang of ["en", "vi"]) expect(fs.existsSync(path.join(dir, `${stem}.${lang}.json`))).toBe(true);
  }
});

test("vi has exactly en's mobile keys, with CLDR plural forms per language", () => {
  const norm = (flat: Record<string, string>) => [...new Set(Object.keys(flat).map(base))].sort();
  expect(norm(V)).toEqual(norm(E));
  for (const [b, f] of forms(E)) expect(f.sort(), `en plural ${b}`).toEqual(["one", "other"]);
  for (const [b, f] of forms(V)) expect(f, `vi plural ${b}`).toEqual(["other"]);
  expect([...forms(V).keys()].sort()).toEqual([...forms(E).keys()].sort());
});

test("en and vi use the same {{variables}}", () => {
  for (const [key, text] of Object.entries(E)) {
    const viKey = PLURAL.test(key) ? `${base(key)}_other` : key;
    expect(vars(V[viKey] ?? ""), key).toEqual(vars(text));
  }
});

const VI_VERB = /^Ghi (?:chú|âm|mới|lại|cuộc gọi|cuộc họp|phòng họp|thử|màn hình|nhận(?! ra| biết)|giọng)/;

test("the brand is {{app}}, never a literal name; no leftover placeholders", () => {
  for (const [key, text] of Object.entries(E)) expect(text, `en ${key}`).not.toMatch(/\bGhi(ra)?\b/);
  for (const [key, text] of Object.entries(V)) {
    expect(text, `vi ${key}`).not.toMatch(/\bGhira\b/);
    for (const m of text.matchAll(/\bGhi\b/g)) {
      expect(VI_VERB.test(text.slice(m.index)), `vi ${key}: "Ghi" must be the verb here`).toBe(true);
    }
  }
  for (const [lang, flat] of [["en", E], ["vi", V]] as const) {
    for (const [key, text] of Object.entries(flat)) {
      expect(text, `${lang} ${key}`).not.toMatch(/\$\{|\{n\}|(?<!\{)\{\w+\}(?!\})/);
      expect(text.trim(), `${lang} ${key}`).not.toBe("");
    }
  }
});

test("mobile keys sit under mobile.* and none redefines a desktop key", () => {
  const desktop = new Set([...Object.keys(flatten(en as Tree)), ...Object.keys(flatten(vi as Tree))]);
  for (const key of [...Object.keys(E), ...Object.keys(V)]) {
    expect(key.startsWith("mobile."), key).toBe(true);
    expect(desktop.has(key), `${key} also exists in the desktop locales`).toBe(false);
  }
});

test("the tab bar and shell placeholders exist", () => {
  for (const key of [
    "mobile.tabs.meetings",
    "mobile.tabs.record",
    "mobile.tabs.search",
    "mobile.tabs.settings",
    "mobile.shell.empty.meetings",
    "mobile.shell.empty.record",
    "mobile.shell.empty.search",
    "mobile.shell.empty.settings",
  ]) {
    expect(E[key], key).toBeTruthy();
    expect(V[key], key).toBeTruthy();
  }
});

test("initMobileI18n serves typed mobile keys next to the desktop ones", () => {
  const i18n = initMobileI18n("vi");
  expect(i18n.t("mobile.tabs.record")).toBe("Ghi âm");
  expect(i18n.t("mobile.ios.share.title")).toBe("Mở trong Ghira");
  expect(i18n.t("mobile.ios.activity.marks", { count: 2 })).toBe("2 dấu");
  expect(i18n.t("common.cancel")).toBeTruthy();
});
