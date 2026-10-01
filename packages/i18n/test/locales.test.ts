// SPDX-License-Identifier: Apache-2.0
// Every locale defines the same keys as English (modulo CLDR plural forms),
// uses the same interpolation variables, never writes the brand into a
// string (it is {{app}}), and has both platform variants of a context key.
import { expect, test } from "vitest";
import en from "../locales/en.json";
import vi from "../locales/vi.json";

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
// `app` is a default variable (the runtime sets it everywhere), and the
// languages may word a sentence with or without the product name.
const vars = (s: string) =>
  [...new Set([...s.matchAll(/\{\{(\w+)\}\}/g)].map((m) => m[1]))].filter((v) => v !== "app").sort();

const E = flatten(en as Tree);
const V = flatten(vi as Tree);

function pluralBases(flat: Record<string, string>): Map<string, string[]> {
  const bases = new Map<string, string[]>();
  for (const key of Object.keys(flat)) {
    if (!PLURAL.test(key)) continue;
    const suffix = key.match(PLURAL)![1];
    bases.set(base(key), [...(bases.get(base(key)) ?? []), suffix]);
  }
  return bases;
}

test("vi has exactly en's keys, with CLDR plural forms per language", () => {
  const norm = (flat: Record<string, string>) => [...new Set(Object.keys(flat).map(base))].sort();
  expect(norm(V)).toEqual(norm(E));

  for (const [b, forms] of pluralBases(E)) {
    expect(forms.sort(), `en plural ${b}`).toEqual(["one", "other"]);
  }
  for (const [b, forms] of pluralBases(V)) {
    expect(forms, `vi plural ${b} (Vietnamese has only "other")`).toEqual(["other"]);
  }
  // a key is plural in both languages or in neither
  expect([...pluralBases(V).keys()].sort()).toEqual([...pluralBases(E).keys()].sort());
});

test("en and vi use the same {{variables}}", () => {
  for (const [key, text] of Object.entries(E)) {
    const viKey = PLURAL.test(key) ? `${base(key)}_other` : key;
    expect(vars(V[viKey] ?? ""), key).toEqual(vars(text));
  }
});

// In Vietnamese "Ghi" is also a verb/noun ("Ghi chú", "Ghi âm", "Ghi mới").
const VI_VERB = /^Ghi (?:chú|âm|mới|lại|cuộc gọi|cuộc họp|phòng họp|thử|màn hình|nhận(?! ra| biết)|giọng)/;

test("the brand is {{app}}, never a literal name; no leftover placeholders", () => {
  for (const [key, text] of Object.entries(E)) {
    expect(text, `en ${key}`).not.toMatch(/\bGhi(ra)?\b/);
  }
  for (const [key, text] of Object.entries(V)) {
    expect(text, `vi ${key}`).not.toMatch(/\bGhira\b/);
    for (const m of text.matchAll(/\bGhi\b/g)) {
      expect(VI_VERB.test(text.slice(m.index)), `vi ${key}: "Ghi" must be the verb here, else use {{app}}`).toBe(true);
    }
  }
  for (const [lang, flat] of [["en", E], ["vi", V]] as const) {
    for (const [key, text] of Object.entries(flat)) {
      expect(text, `${lang} ${key}`).not.toMatch(/\$\{|\{n\}|(?<!\{)\{\w+\}(?!\})/);
    }
  }
});

test("every _mac key has a _win sibling and vice versa", () => {
  for (const [lang, flat] of [["en", E], ["vi", V]] as const) {
    const keys = new Set(Object.keys(flat).map(base));
    for (const k of keys) {
      if (k.endsWith("_mac")) expect(keys, `${lang} ${k}`).toContain(k.replace(/_mac$/, "_win"));
      if (k.endsWith("_win")) expect(keys, `${lang} ${k}`).toContain(k.replace(/_win$/, "_mac"));
    }
  }
});
