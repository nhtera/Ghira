// SPDX-License-Identifier: Apache-2.0
// One-shot extraction of the UI copy from the Claude Design export into
// i18next JSON (v4). After it has run, the committed locales/*.json are the
// source of truth; this script only exists to make the first import auditable.
//
//   node packages/i18n/scripts/extract-from-design.mjs <design-dir>
//
// <design-dir> holds ghi-data.js and ghi-data-2.js (Plans/ is local only, so
// CI never runs this). Every leaf in both files must be classified in
// design-manifest.mjs (copy -> key, sample data -> mocks, prototype-only ->
// ignore); an unclassified or stale entry fails the run. Output is
// deterministic.
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";
import { MANIFEST } from "./design-manifest.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const i18nDir = path.resolve(here, "..");
const repoRoot = path.resolve(i18nDir, "../..");
const outLocales = path.join(i18nDir, "locales");
const outMocks = path.join(repoRoot, "packages/ui/mocks");
const outReview = path.join(repoRoot, "plans/reports/i18n-extract-261001-brand-review.md");

const designDir = process.argv[2];
if (!designDir || !fs.existsSync(path.join(designDir, "ghi-data.js"))) {
  console.log(
    "extract-from-design: design export not found" +
      (designDir ? ` at ${designDir}` : "") +
      "; nothing to do (locales/*.json are the source of truth).",
  );
  process.exit(0);
}

// ---- load the design data in a sandbox ------------------------------------

function loadDesign(dir) {
  const queue = [];
  const window = {};
  const sandbox = {
    window,
    setInterval: (f) => (queue.push(f), 1),
    clearInterval: () => {},
  };
  vm.createContext(sandbox);
  for (const f of ["ghi-data.js", "ghi-data-2.js"]) {
    vm.runInContext(fs.readFileSync(path.join(dir, f), "utf8"), sandbox, { filename: f });
  }
  queue.forEach((f) => f());
  return window.GHI;
}

const G = loadDesign(designDir);

// ---- state -----------------------------------------------------------------

const flat = { en: new Map(), vi: new Map() };
const mocks = {};
const log = { ignored: [], dropped: [], brandVi: [], win: [], aliases: [], macOnly: [] };
const pendingAliases = [];
const RAW = Symbol("raw");

const fail = (msg) => {
  throw new Error(`extract-from-design: ${msg}`);
};
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

// ---- templates ---------------------------------------------------------------

function pickForm(t, lang, vars) {
  if (typeof t === "string") return t;
  const n = vars && vars.count;
  return lang === "en" && n === 1 ? t.one : t.other;
}

function render(t, lang, vars, where) {
  const s = pickForm(t, lang, vars);
  const r = s.replace(/\{\{(\w+)\}\}/g, (_, x) => {
    if (!(x in vars)) fail(`${where}: variable {{${x}}} has no check value`);
    return String(vars[x]);
  });
  if (r.includes("{{")) fail(`${where}: unresolved placeholder in "${s}"`);
  return r;
}

const langVars = (vars, lang) => (vars && ("en" in vars || "vi" in vars) ? vars[lang] : vars);

// ---- brand -------------------------------------------------------------------
// EN: "Ghi" is always the product. VI: "Ghi" is also a verb/noun ("Ghi chú",
// "ghi âm", "Ghi mới", "Ghi lại"); keep it where it is the verb.

const VI_VERB = /^Ghi (?:chú|âm|mới|lại|cuộc gọi|cuộc họp|phòng họp|thử|màn hình|nhận(?! ra| biết)|giọng)/;

function brandEn(s) {
  return s.replace(/\bGhi\b/g, "{{app}}");
}

function brandVi(s, key) {
  return s.replace(/\bGhi\b/g, (m, i) => {
    const verb = VI_VERB.test(s.slice(i));
    log.brandVi.push({
      key,
      snippet: s.slice(Math.max(0, i - 14), i + 22).replace(/\n/g, " "),
      decision: verb ? "kept (verb/noun)" : "{{app}} (product)",
    });
    return verb ? m : "{{app}}";
  });
}

const mapForms = (t, fn) =>
  typeof t === "string" ? fn(t) : Object.fromEntries(Object.entries(t).map(([k, v]) => [k, fn(v)]));

// ---- platform ------------------------------------------------------------------

const PLATFORM_EN = /\b(Mac|macOS|MacBook|Dock|menu bar|Touch ID|Keychain|System Settings)\b/;
const PLATFORM_VI = /(Mac\b|macOS|Dock|thanh menu|Touch ID|Keychain|Cài đặt hệ thống)/;

function deriveWin(s) {
  return s.replace(/this Mac/g, "this PC").replace(/macOS/g, "Windows");
}

// ---- output ---------------------------------------------------------------------

function put(lang, key, value) {
  const m = flat[lang];
  if (m.has(key) && m.get(key) !== value) fail(`key ${lang}:${key} defined twice with different text`);
  m.set(key, value);
}

function emit(base, forms) {
  const plural = typeof forms.en !== "string";
  if (typeof forms.vi !== "string") fail(`${base}: vi form must be a single string (Vietnamese has only "other")`);
  if (plural) {
    if (!forms.en.one || !forms.en.other || forms.en.one === forms.en.other) {
      fail(`${base}: en plural needs distinct one/other`);
    }
    put("en", `${base}_one`, forms.en.one);
    put("en", `${base}_other`, forms.en.other);
    put("vi", `${base}_other`, forms.vi);
  } else {
    put("en", base, forms.en);
    put("vi", base, forms.vi);
  }
}

// ---- leaves -------------------------------------------------------------------------

function setMock(spec, value, where) {
  const [file, sub] = spec.split("#");
  if (!sub) fail(`${where}: mock spec "${spec}" needs file#path`);
  const doc = (mocks[file] ??= {});
  const parts = sub.split(".");
  let o = doc;
  for (const p of parts.slice(0, -1)) o = o[p] ??= {};
  const last = parts[parts.length - 1];
  if (last in o && !eq(o[last], value)) fail(`${where}: mock ${spec} defined twice`);
  o[last] = value;
}

function normalize(d) {
  if (typeof d === "string") {
    if (d.startsWith("@")) return { ignore: d.slice(1).trim() || fail("ignore needs a reason") };
    if (d.startsWith("~")) return { mock: d.slice(1) };
    if (d.startsWith(">")) return { k: d.slice(1), alias: true };
    return { k: d };
  }
  return d;
}

function leaf(d, en, vi, where) {
  const o = normalize(d);
  const preview = (typeof en === "function" ? "(fn)" : JSON.stringify(en)).slice(0, 70);
  if (o.ignore !== undefined) {
    const kind = o.ignore.startsWith("icon:") || o.ignore.startsWith("id:") ? "dropped" : "ignored";
    log[kind].push({ where, reason: o.ignore, preview });
    return;
  }
  if (o.mock !== undefined) {
    if (typeof en === "function") fail(`${where}: cannot mock a function`);
    setMock(o.mock, vi === RAW || eq(en, vi) ? en : { en, vi }, where);
    return;
  }
  if (!o.k) fail(`${where}: bad directive ${JSON.stringify(d)}`);
  if (vi === RAW) fail(`${where}: copy leaf in a non-paired section`);

  const isFn = typeof en === "function";
  if (isFn !== (typeof vi === "function")) fail(`${where}: en/vi differ in kind`);
  if (!isFn && (typeof en !== "string" || typeof vi !== "string")) {
    fail(`${where}: expected string leaf, got ${JSON.stringify(en)}`);
  }
  const calls = o.calls ?? [];
  const samples = isFn
    ? ["en", "vi"].map((l) => calls.map((c) => (l === "en" ? en : vi)(...c.args)))
    : [[en], [vi]];
  if (isFn && !calls.length) fail(`${where}: function leaf needs calls`);

  // 1. template
  let tpl;
  if (o.tpl) {
    tpl = [o.tpl[0], o.tpl[1]];
  } else {
    if (isFn) fail(`${where}: function leaf needs tpl`);
    const v = o.v || {};
    const conv = (s) => s.replace(/\{(\w+)\}/g, (_, x) => `{{${v[x] || x}}}`);
    const subs = o.sub ? (Array.isArray(o.sub) ? { en: o.sub, vi: o.sub } : o.sub) : { en: [], vi: [] };
    const apply = (s, list) => {
      for (const [a, b] of list) {
        if (!s.includes(a)) fail(`${where}: sub "${a}" not found in "${s}"`);
        s = s.replace(a, b);
      }
      return s;
    };
    tpl = [apply(conv(en), subs.en), apply(conv(vi), subs.vi)];
    if (o.one) {
      if (!tpl[0].includes(o.one[0])) fail(`${where}: one "${o.one[0]}" not found`);
      tpl[0] = { one: tpl[0].replace(o.one[0], o.one[1]), other: tpl[0] };
    }
  }

  // 2. verify the template reproduces the design string(s)
  if (isFn) {
    for (const [i, c] of calls.entries()) {
      for (const [li, l] of ["en", "vi"].entries()) {
        const got = render(tpl[li], l, c.vars, where);
        if (got !== samples[li][i]) fail(`${where}: ${l} tpl renders "${got}", design says "${samples[li][i]}"`);
      }
    }
  } else if (o.vars || o.tpl || o.sub) {
    for (const [li, l] of ["en", "vi"].entries()) {
      const vars = langVars(o.vars, l) ?? {};
      const got = render(tpl[li], l, vars, where);
      if (got !== samples[li][0]) fail(`${where}: ${l} tpl renders "${got}", design says "${samples[li][0]}"`);
    }
  }
  if (o.mockVars) setMock(o.mockVars, o.vars, where);

  // 3. platform guard on the design text
  const text = (l) => samples[l === "en" ? 0 : 1];
  if (!o.plat && !o.ctx && !o.macOnly && !o.neutral && !o.alias) {
    if (text("en").some((s) => PLATFORM_EN.test(s)) || text("vi").some((s) => PLATFORM_VI.test(s))) {
      fail(`${where}: platform-specific copy needs plat/ctx/macOnly: ${JSON.stringify(samples[0][0])}`);
    }
  }
  if (o.macOnly) log.macOnly.push({ k: o.k, en: samples[0][0] });

  // 4. brand
  const key = o.k;
  const forms = {
    en: mapForms(tpl[0], brandEn),
    vi: mapForms(tpl[1], (s) => brandVi(s, key)),
  };
  for (const s of typeof forms.en === "string" ? [forms.en] : Object.values(forms.en)) {
    if (/\bGhi(ra)?\b/.test(s)) fail(`${where}: brand left in en "${s}"`);
  }

  // 5. alias check (deferred) or emit
  if (o.alias) {
    pendingAliases.push({ where, key, forms });
    return;
  }
  if (o.plat) {
    let win;
    if (o.plat === true) {
      win = { en: mapForms(forms.en, deriveWin), vi: mapForms(forms.vi, deriveWin) };
      if (eq(win.en, forms.en)) fail(`${where}: plat:true but nothing to derive for en`);
    } else {
      win = {
        en: mapForms(o.plat.en, brandEn),
        vi: mapForms(o.plat.vi, (s) => brandVi(s, `${key}_win`)),
      };
    }
    emit(`${key}_mac`, forms);
    emit(`${key}_win`, win);
    log.win.push({ key, mode: o.plat === true ? "rule" : "explicit", mac: forms, win });
  } else if (o.ctx) {
    emit(`${key}_${o.ctx}`, forms);
  } else {
    emit(key, forms);
  }
  if (o.also) leaf(o.also, en, vi, `${where}+also`);
}

// ---- tree walk ---------------------------------------------------------------------------

function visit(d, en, vi, where) {
  const isNode =
    d && typeof d === "object" && !Array.isArray(d) && d.k === undefined && d.ignore === undefined && d.mock === undefined;
  if (Array.isArray(d)) {
    if (!Array.isArray(en) || (vi !== RAW && !Array.isArray(vi)) || en.length !== d.length || (vi !== RAW && vi.length !== d.length)) {
      fail(`${where}: manifest has ${d.length} entries, data has ${Array.isArray(en) ? en.length : "non-array"}`);
    }
    d.forEach((c, i) => visit(c, en[i], vi === RAW ? RAW : vi[i], `${where}[${i}]`));
  } else if (isNode) {
    const pairs = d.$pairs === true;
    const keys = Object.keys(d).filter((k) => k !== "$pairs");
    if (en === null || typeof en !== "object") fail(`${where}: expected an object`);
    const dataKeys = Object.keys(en);
    if (!pairs && vi !== RAW && !eq([...dataKeys].sort(), Object.keys(vi).sort())) {
      fail(`${where}: en and vi have different keys`);
    }
    for (const k of dataKeys) if (!keys.includes(k)) fail(`UNCLASSIFIED ${where}.${k}`);
    for (const k of keys) if (!dataKeys.includes(k)) fail(`stale manifest entry ${where}.${k}`);
    for (const k of keys) {
      if (pairs && typeof d[k] === "string" && d[k].startsWith("@")) {
        visit(d[k], en[k], RAW, `${where}.${k}`);
      } else if (pairs) {
        if (!Array.isArray(en[k]) || en[k].length !== 2) {
          fail(`${where}.${k}: expected an [EN, VI] pair, got ${JSON.stringify(en[k]).slice(0, 60)}`);
        }
        visit(d[k], en[k][0], en[k][1], `${where}.${k}`);
      } else {
        visit(d[k], en[k], vi === RAW ? RAW : vi[k], `${where}.${k}`);
      }
    }
  } else {
    leaf(d, en, vi, where);
  }
}

// Top level: T is {en, vi}; every other global is classified on its own.
const manifest = MANIFEST;
for (const k of Object.keys(G)) if (!(k in manifest)) fail(`UNCLASSIFIED top-level ${k}`);
for (const k of Object.keys(manifest)) if (!(k in G)) fail(`stale manifest entry ${k}`);
for (const k of Object.keys(manifest)) {
  if (k === "T" || k === "CONSENT") {
    if (!eq(Object.keys(G[k]).sort(), ["en", "vi"])) fail(`${k}: expected {en, vi}`);
    visit(manifest[k], G[k].en, G[k].vi, k);
  } else visit(manifest[k], G[k], RAW, k);
}

// aliases: same text as an existing key
for (const a of pendingAliases) {
  const base = flat.en.has(a.key) ? a.key : `${a.key}_mac`;
  if (!flat.en.has(base)) fail(`${a.where}: alias target ${a.key} does not exist`);
  const want = { en: flat.en.get(base), vi: flat.vi.get(base) };
  if (want.en !== a.forms.en || want.vi !== a.forms.vi) {
    fail(`${a.where}: alias ${a.key} differs: ${JSON.stringify(a.forms)} vs ${JSON.stringify(want)}`);
  }
  log.aliases.push(`${a.where} -> ${a.key}`);
}

// existing hand-written entries that stay (the runtime owns APP_NAME)
put("en", "app.tagline", "Private meeting notes, on your device.");
put("vi", "app.tagline", "Ghi chú cuộc họp riêng tư, ngay trên máy bạn.");

// ---- sanity -----------------------------------------------------------------------------------

for (const lang of ["en", "vi"]) {
  for (const [k, v] of flat[lang]) {
    if (/\$\{|\{n\}|\{m\}|\{\w\}/.test(v)) fail(`${lang}:${k} has a leftover placeholder: ${v}`);
  }
}

function nest(map) {
  const root = {};
  for (const [key, value] of map) {
    const parts = key.split(".");
    let o = root;
    for (const p of parts.slice(0, -1)) {
      if (typeof o[p] === "string") fail(`key ${key}: ${p} is both a leaf and a group`);
      o = o[p] ??= {};
    }
    const last = parts[parts.length - 1];
    if (typeof o[last] === "object") fail(`key ${key} is both a leaf and a group`);
    o[last] = value;
  }
  return root;
}

const write = (file, data) => {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify(data, null, 2) + "\n");
};

write(path.join(outLocales, "en.json"), nest(flat.en));
write(path.join(outLocales, "vi.json"), nest(flat.vi));
for (const [file, data] of Object.entries(mocks)) write(path.join(outMocks, file), data);

// ---- review list ---------------------------------------------------------------------------------

const md = [];
md.push("# i18n extraction review list", "", "Generated by `packages/i18n/scripts/extract-from-design.mjs`. A human should check each section.", "");
md.push("## 1. Vietnamese brand decisions (\"Ghi\")", "");
md.push("Rule: `{{app}}` only when \"Ghi\" is the product; keep it when it is the verb/noun (Ghi chú, Ghi âm, Ghi mới, Ghi lại, Ghi thử, Ghi cuộc gọi/họp, Ghi phòng họp, Ghi màn hình, Ghi giọng). Lowercase \"ghi\" is always the verb and untouched.", "");
md.push("| Key | Context | Decision |", "| --- | --- | --- |");
for (const r of log.brandVi) md.push(`| \`${r.key}\` | …${r.snippet.replace(/\|/g, "\\|")}… | ${r.decision} |`);
md.push("", "## 2. Windows variants (`_win`) that the design did not provide", "");
md.push("`rule` = EN \"this Mac\" -> \"this PC\", \"macOS\" -> \"Windows\"; VI unchanged (\"máy này\" is already platform-neutral). `explicit` = hand-written.", "");
md.push("| Key | Mode | EN win | VI win |", "| --- | --- | --- | --- |");
const cell = (t) => (typeof t === "string" ? t : t.other).replace(/\|/g, "\\|");
for (const w of log.win) md.push(`| \`${w.key}\` | ${w.mode} | ${cell(w.win.en)} | ${cell(w.win.vi)} |`);
md.push("", "## 3. macOS-only strings kept without a `_win` sibling", "");
for (const m of log.macOnly) md.push(`- \`${m.k}\`: ${m.en}`);
md.push("", "## 4. Ignored (prototype-only / demo controls / design tokens)", "");
for (const i of log.ignored) md.push(`- \`${i.where}\` ${i.preview}: ${i.reason}`);
md.push("", "## 5. Dropped icon ids and ids (derived from the key names in code)", "");
const droppedBy = new Map();
for (const i of log.dropped) droppedBy.set(i.reason, (droppedBy.get(i.reason) ?? 0) + 1);
for (const [r, n] of droppedBy) md.push(`- ${r}: ${n}`);
md.push("", "## 6. Strings that alias an existing key (verified identical)", "");
for (const a of log.aliases) md.push(`- ${a}`);
md.push("");
fs.mkdirSync(path.dirname(outReview), { recursive: true });
fs.writeFileSync(outReview, md.join("\n"));

const count = (m) => m.size;
console.log(
  `extract-from-design: wrote en (${count(flat.en)} keys), vi (${count(flat.vi)} keys), ` +
    `${Object.keys(mocks).length} mock files, review list at ${path.relative(repoRoot, outReview)}`,
);
