// SPDX-License-Identifier: Apache-2.0
// One-shot extraction of the mobile UI copy (the `P` dict, EN/VI, in the
// Claude Design export's "Ghi Mobile.dc.html") into
// locales/mobile/_base.{en,vi}.json. After it has run, the committed files are
// the source of truth; this only makes the import auditable.
//
//   node packages/i18n/scripts/extract-mobile.mjs <design-dir>
//
// Plans/ is local only, so CI never runs this. Every entry of the dict must
// be classified in mobile-manifest.mjs; an unclassified or stale entry fails
// the run. Output is deterministic. Does not touch locales/en.json or vi.json.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { MOBILE_MANIFEST } from "./mobile-manifest.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const outDir = path.resolve(here, "../locales/mobile");
const designDir = process.argv[2];
const file = designDir && path.join(designDir, "Ghi Mobile.dc.html");
if (!file || !fs.existsSync(file)) {
  console.log(
    "extract-mobile: design export not found" +
      (designDir ? ` at ${designDir}` : "") +
      "; nothing to do (locales/mobile/_base.*.json are the source of truth).",
  );
  process.exit(0);
}

const fail = (msg) => {
  throw new Error(`extract-mobile: ${msg}`);
};

// ---- load the P dict -------------------------------------------------------

function loadDict(html) {
  const a = html.indexOf("const P = {");
  const b = html.indexOf("\nclass Component", a);
  if (a < 0 || b < 0) fail("cannot find the `const P = {...}` dict");
  // The dict is a plain object literal of strings and arrays.
  return new Function(`return (${html.slice(a + "const P = ".length, b).trim().replace(/;$/, "")})`)();
}

const P = loadDict(fs.readFileSync(file, "utf8"));

// ---- brand -------------------------------------------------------------------
// EN: "Ghi" is always the product. VI: "Ghi" is also the verb in "Ghi âm",
// "Ghi chú", "Ghi giọng"; keep it where it is the verb (as extract-from-design).

const VI_VERB = /^Ghi (?:chú|âm|mới|lại|cuộc gọi|cuộc họp|phòng họp|thử|màn hình|nhận(?! ra| biết)|giọng)/;
const brandEn = (s) => s.replace(/\bGhi\b/g, "{{app}}");
const brandVi = (s) => s.replace(/\bGhi\b/g, (m, i) => (VI_VERB.test(s.slice(i)) ? m : "{{app}}"));

// ---- directives -----------------------------------------------------------------

const flat = { en: new Map(), vi: new Map() };
const aliases = [];
const ignored = [];

function put(key, en, vi, where) {
  if (flat.en.has(key)) fail(`${where}: key ${key} defined twice`);
  flat.en.set(key, en);
  flat.vi.set(key, vi);
}

const vars = (s) => [...new Set([...s.matchAll(/\{\{(\w+)\}\}/g)].map((m) => m[1]))].filter((v) => v !== "app").sort();

function convert(dir, en, vi, where) {
  const opts = typeof dir === "string" ? { k: dir } : dir;
  let outEn = en;
  let outVi = vi;
  if (opts.tpl) {
    [outEn, outVi] = opts.tpl;
  } else {
    const ren = opts.v ?? {};
    const rename = (s) => s.replace(/\{(\w+)\}/g, (_, x) => `{{${ren[x] ?? x}}}`);
    outEn = rename(outEn);
    outVi = rename(outVi);
    const sub = opts.sub ?? [];
    for (const [lang, [from, to]] of sub.flatMap((p) => [["en", p], ["vi", p]])) {
      const cur = lang === "en" ? outEn : outVi;
      if (!cur.includes(from)) fail(`${where}: "${from}" not found in ${lang} text "${cur}"`);
      if (lang === "en") outEn = outEn.replace(from, to);
      else outVi = outVi.replace(from, to);
    }
    outEn = brandEn(outEn);
    outVi = brandVi(outVi);
  }
  if (/(?<!\{)\{\w+\}(?!\})/.test(outEn + outVi)) fail(`${where}: single-brace placeholder left in "${outEn}" / "${outVi}"`);
  if (vars(outEn).join() !== vars(outVi).join()) {
    fail(`${where}: en and vi use different variables (${vars(outEn)} vs ${vars(outVi)})`);
  }
  return [opts.k, outEn, outVi];
}

function apply(dir, en, vi, where) {
  if (typeof dir === "string" && dir.startsWith("@")) {
    ignored.push(`${where}: ${dir.slice(1)}`);
    return;
  }
  if (typeof dir === "string" && dir.startsWith(">")) {
    aliases.push({ where, key: dir.slice(1), en, vi });
    return;
  }
  if (typeof en !== "string" || typeof vi !== "string") fail(`${where}: expected a string pair`);
  const [k, e, v] = convert(dir, en, vi, where);
  put(k, e, v, where);
}

for (const name of Object.keys(P)) {
  if (!(name in MOBILE_MANIFEST)) fail(`unclassified entry "${name}" (add it to mobile-manifest.mjs)`);
}
for (const [name, dir] of Object.entries(MOBILE_MANIFEST)) {
  if (!(name in P)) fail(`manifest entry "${name}" is not in the design`);
  const [en, vi] = P[name];
  if (dir && typeof dir === "object" && dir.items) {
    if (!Array.isArray(en) || en.length !== dir.items.length || vi.length !== en.length) {
      fail(`${name}: expected ${dir.items.length} items in both languages`);
    }
    dir.items.forEach((d, i) => apply(d, en[i], vi[i], `${name}[${i}]`));
  } else {
    apply(dir, en, vi, name);
  }
}
for (const a of aliases) {
  if (!flat.en.has(a.key)) fail(`${a.where}: alias target ${a.key} does not exist`);
  const [, e, v] = convert(a.key, a.en, a.vi, a.where);
  if (flat.en.get(a.key) !== e || flat.vi.get(a.key) !== v) fail(`${a.where}: differs from ${a.key}`);
}

// ---- write ----------------------------------------------------------------------

function nest(map) {
  const root = {};
  for (const key of [...map.keys()].sort()) {
    const parts = ["mobile", ...key.split(".")];
    let node = root;
    for (const p of parts.slice(0, -1)) {
      if (typeof node[p] === "string") fail(`key ${key} collides with a leaf`);
      node = node[p] ??= {};
    }
    const leaf = parts.at(-1);
    if (leaf in node) fail(`key ${key} collides with a namespace`);
    node[leaf] = map.get(key);
  }
  return root;
}

fs.mkdirSync(outDir, { recursive: true });
for (const lang of ["en", "vi"]) {
  fs.writeFileSync(path.join(outDir, `_base.${lang}.json`), JSON.stringify(nest(flat[lang]), null, 2) + "\n");
}
console.log(`extract-mobile: ${flat.en.size} keys written, ${ignored.length} entries ignored, ${aliases.length} aliases`);
