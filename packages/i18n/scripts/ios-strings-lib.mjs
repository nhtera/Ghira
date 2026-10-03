// SPDX-License-Identifier: Apache-2.0
// The string rules behind gen-ios-strings.mjs (kept apart so tests can import them).

export const APP = "Ghira";
export const PREFIX = "mobile.ios.";
export const PLURAL = /_(zero|one|two|few|many|other)$/;

/** The {{vars}} of a string except {{app}}, in order of first appearance. */
export function varsOf(s) {
  const out = [];
  for (const [, v] of s.matchAll(/\{\{(\w+)\}\}/g)) if (v !== "app" && !out.includes(v)) out.push(v);
  return out;
}

/**
 * One localized string for Swift's String(format:). `order` is the variable
 * order of the EN string, so VI (which may reorder words) keeps the same
 * positions. {{app}} is the product name; {{count}} is %lld, other variables
 * %@. A string with two or more variables uses positional specifiers
 * (%1$@, %2$lld). A literal % becomes %%.
 */
export function format(s, order) {
  const positional = order.length > 1;
  return s
    .replace(/%/g, "%%")
    .replace(/\{\{(\w+)\}\}/g, (_, v) => {
      if (v === "app") return APP;
      const spec = v === "count" ? "lld" : "@";
      if (!positional) return `%${spec}`;
      return `%${order.indexOf(v) + 1}$${spec}`;
    });
}

const unit = (value) => ({ stringUnit: { state: "translated", value } });

/** The .xcstrings JSON text for flattened `{ key: text }` maps per language. */
export function build(langs) {
  const keys = new Map(); // full key -> plural?
  for (const key of Object.keys(langs.en)) {
    if (!key.startsWith(PREFIX)) continue;
    const plural = PLURAL.test(key);
    keys.set(plural ? key.replace(PLURAL, "") : key, plural);
  }
  const strings = {};
  for (const key of [...keys.keys()].sort()) {
    const localizations = {};
    for (const lang of ["en", "vi"]) {
      if (keys.get(key)) {
        const variations = {};
        for (const form of ["zero", "one", "two", "few", "many", "other"]) {
          const text = langs[lang][`${key}_${form}`];
          if (text !== undefined) {
            const order = varsOf(langs.en[`${key}_other`] ?? "");
            variations[form] = unit(format(text, order));
          }
        }
        if (!variations.other) throw new Error(`gen-ios-strings: ${key} (${lang}) has no "other" form`);
        localizations[lang] = { variations: { plural: variations } };
      } else {
        const text = langs[lang][key];
        if (text === undefined) throw new Error(`gen-ios-strings: ${key} missing in ${lang}`);
        localizations[lang] = unit(format(text, varsOf(langs.en[key])));
      }
    }
    strings[key] = { extractionState: "manual", localizations };
  }
  return JSON.stringify({ sourceLanguage: "en", strings, version: "1.0" }, null, 2) + "\n";
}
