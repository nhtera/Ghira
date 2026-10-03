// SPDX-License-Identifier: Apache-2.0
// Generates native/ios/Shared/Localizable.xcstrings (EN + VI) from the
// `mobile.ios.*` keys in locales/mobile, so the Live Activity and the share
// extension (Swift) read the same copy as the app.
//
//   node packages/i18n/scripts/gen-ios-strings.mjs [--check]
//
// --check fails if the committed file is out of date. Swift looks a string
// up by its full key, e.g. String(localized: "mobile.ios.activity.phase.live").
// The string rules (positional %N$@ / %N$lld when a string has several
// variables, %% for a literal %) are in ios-strings-lib.mjs.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { build } from "./ios-strings-lib.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const localesDir = path.join(here, "../locales/mobile");
const outFile = path.resolve(here, "../../../native/ios/Shared/Localizable.xcstrings");

function flatten(tree, prefix = "") {
  const out = {};
  for (const [k, v] of Object.entries(tree)) {
    if (typeof v === "string") out[`${prefix}${k}`] = v;
    else Object.assign(out, flatten(v, `${prefix}${k}.`));
  }
  return out;
}

function load(lang) {
  const merged = {};
  for (const f of fs.readdirSync(localesDir).sort()) {
    if (!f.endsWith(`.${lang}.json`)) continue;
    for (const [k, v] of Object.entries(flatten(JSON.parse(fs.readFileSync(path.join(localesDir, f), "utf8"))))) {
      if (k in merged) throw new Error(`gen-ios-strings: ${k} defined twice`);
      merged[k] = v;
    }
  }
  return merged;
}

const out = build({ en: load("en"), vi: load("vi") });
if (process.argv.includes("--check")) {
  const current = fs.existsSync(outFile) ? fs.readFileSync(outFile, "utf8") : "";
  if (current !== out) {
    console.error("native/ios/Shared/Localizable.xcstrings is stale: run `pnpm --filter @ghi/i18n gen:ios-strings`");
    process.exit(1);
  }
  console.log("gen-ios-strings: up to date");
} else {
  fs.mkdirSync(path.dirname(outFile), { recursive: true });
  fs.writeFileSync(outFile, out);
  console.log(`gen-ios-strings: wrote ${path.relative(process.cwd(), outFile)}`);
}
