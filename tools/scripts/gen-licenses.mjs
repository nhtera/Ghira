// SPDX-License-Identifier: Apache-2.0
// Generates apps/desktop/src/generated/licenses.json for Settings -> About ->
// Licenses (Rust crates, bundled npm packages, models, bundled assets).
// Run from the repo root: `node tools/scripts/gen-licenses.mjs` (or `pnpm gen:licenses`).
// Needs cargo-about and a prior `pnpm build` (apps/desktop/dist/third-party-js.json).
// `--app mobile` (pnpm gen:licenses:mobile) does the same for the iPhone app: the
// aarch64-apple-ios crates of apps/mobile/src-tauri and apps/mobile/dist.
// Output is deterministic: no timestamps, everything sorted.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));
const app = process.argv.includes("--app") ? process.argv[process.argv.indexOf("--app") + 1] : "desktop";
if (app !== "desktop" && app !== "mobile") throw new Error(`unknown --app ${app}`);
const out = join(root, `apps/${app}/src/generated/licenses.json`);
const read = (p) => readFileSync(join(root, p), "utf8");
const byName = (a, b) =>
  a.name < b.name ? -1 : a.name > b.name ? 1 : a.version < b.version ? -1 : a.version > b.version ? 1 : 0;

// ---- license text store (deduped modulo whitespace and copyright lines) ----
const COPYRIGHT = /^\s*(copyright|\(c\))\s+(\(c\)\s*|©\s*)?(19|20)\d\d/i;
const licenses = {};

/** Registers a text, returns { key, copyright } (the copyright lines found in it). */
function addText(id, name, raw) {
  const lines = raw.replace(/\r\n?/g, "\n").split("\n");
  const copyright = lines.filter((l) => COPYRIGHT.test(l)).map((l) => l.trim());
  const body = lines.filter((l) => !COPYRIGHT.test(l)).join("\n").replace(/\n{3,}/g, "\n\n").trim();
  const norm = body.replace(/\s+/g, " ").toLowerCase();
  const key = `${id}-${createHash("sha1").update(norm).digest("hex").slice(0, 6)}`;
  licenses[key] ??= { id, name, text: body };
  return { key, copyright };
}

const uniqSorted = (xs) => [...new Set(xs)].sort();

// ---- 1. Rust ----
// about.toml is the CI config; webpki-root-certs (CDLA-Permissive-2.0) is allowed
// by deny.toml but missing there, so add it to a temporary copy.
function rustSection() {
  let cfg = read("about.toml");
  if (!cfg.includes("CDLA-Permissive-2.0")) cfg = cfg.replace(/accepted = \[/, 'accepted = [\n  "CDLA-Permissive-2.0",');
  // The phone ships the iOS build only: scope the crate graph to its target.
  if (app === "mobile") cfg += '\ntargets = ["aarch64-apple-ios", "aarch64-apple-ios-sim"]\n';
  const tmp = join(mkdtempSync(join(tmpdir(), "ghi-about-")), "about.toml");
  writeFileSync(tmp, cfg);
  const json = JSON.parse(
    execFileSync(
      "cargo",
      ["about", "generate", "--format", "json", "-c", tmp, "--manifest-path", `apps/${app}/src-tauri/Cargo.toml`],
      { cwd: root, encoding: "utf8", maxBuffer: 256 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] },
    ),
  );
  const own = (n) => /^ghi[-_]/.test(n);
  const crates = new Map();
  for (const c of json.crates) {
    if (own(c.package.name)) continue;
    crates.set(c.package.id, {
      name: c.package.name,
      version: c.package.version,
      license: c.license,
      keys: [],
      copyright: [],
    });
  }
  for (const l of json.licenses) {
    const { key, copyright } = addText(l.id, l.name, l.text);
    for (const u of l.used_by) {
      const c = crates.get(u.crate.id);
      if (!c) continue;
      c.keys.push(key);
      c.copyright.push(...copyright);
    }
  }
  return [...crates.values()]
    .map((c) => ({
      name: c.name,
      version: c.version,
      license: c.license,
      licenseKeys: uniqSorted(c.keys),
      ...(c.copyright.length ? { copyright: uniqSorted(c.copyright) } : {}),
    }))
    .sort(byName);
}

// ---- 2. JavaScript bundled into the desktop frontend ----
function jsSection() {
  const list = new URL(`apps/${app}/dist/third-party-js.json`, `file://${root}`);
  if (!existsSync(list)) throw new Error(`apps/${app}/dist/third-party-js.json is missing: run \`pnpm build\` first.`);
  const pnpm = join(root, "node_modules/.pnpm");
  const dirs = readdirSync(pnpm);
  return JSON.parse(readFileSync(list, "utf8"))
    .map((p) => {
      const prefix = `${p.name.replace("/", "+")}@${p.version}`;
      const dir = dirs.find((d) => d === prefix || d.startsWith(`${prefix}_`));
      const pkgDir = dir && join(pnpm, dir, "node_modules", p.name);
      const file = pkgDir && existsSync(pkgDir) && readdirSync(pkgDir).find((f) => /^(licen[sc]e|copying)(\..*)?$/i.test(f));
      const entry = { name: p.name, version: p.version, license: p.license, licenseKeys: [] };
      if (!file) {
        // No license file in the package: fall back to the stored text for that id.
        entry.licenseKeys = Object.entries(licenses).filter(([, l]) => l.id === p.license).map(([k]) => k).sort().slice(0, 1);
      } else {
        const { key, copyright } = addText(p.license, p.license, readFileSync(join(pkgDir, file), "utf8"));
        entry.licenseKeys = [key];
        if (copyright.length) entry.copyright = uniqSorted(copyright);
      }
      return entry;
    })
    .sort(byName);
}

// ---- 2b. Native components (C/C++ that cargo-about cannot see; third_party/NATIVE_NOTICES.md) ----
// Texts are read verbatim (copyright lines and NOTICE files are part of the
// license obligation, so addText's de-duplication is not used). The SentencePiece,
// SQLCipher, libopus and OpenSSL texts are vendored in third_party/licenses/
// (their sources only exist inside the build tree / the cargo registry).
const NEMO = "third_party/NeMo-Speech.cpp";
const WHISPER = "third_party/whisper.cpp";
const VENDORED = "third_party/licenses";

function addVerbatim(id, name, raw) {
  const text = raw.replace(/\r\n?/g, "\n").trim();
  const key = `${id}-${createHash("sha1").update(text).digest("hex").slice(0, 6)}`;
  licenses[key] ??= { id, name, text };
  return key;
}

/** The "## MIT License" section at the end of NeMo-Speech.cpp's THIRD_PARTY_NOTICES.md. */
const nemoMit = () => read(`${NEMO}/THIRD_PARTY_NOTICES.md`).split(/^## MIT License\s*$/m)[1];
const parakeetNotice = () => read(`${NEMO}/THIRD_PARTY_NOTICES.md`).match(/^### parakeet\.cpp\n[\s\S]*?(?=^### )/m)[0];

function nativeSection() {
  const nemoVersion = read(`${NEMO}/VERSION`).match(/(\d+\.\d+\.\d+)/)[1];
  const sp = "https://github.com/google/sentencepiece";
  const all = [
    { name: "NeMo-Speech.cpp (NVIDIA)", version: nemoVersion, license: "Apache-2.0", url: "https://github.com/NVIDIA-NeMo/NeMo-Speech.cpp",
      texts: [["Apache-2.0", "Apache License 2.0", () => read(`${NEMO}/LICENSE`)], ["NOTICE", "NeMo-Speech.cpp NOTICE", () => read(`${NEMO}/NOTICE`)]] },
    { name: "ggml (with NVIDIA's patches)", version: "pinned by NeMo-Speech.cpp", license: "MIT", url: "https://github.com/ggml-org/ggml",
      texts: [["MIT", "MIT License (ggml)", () => read(`${NEMO}/ggml/LICENSE`)]] },
    { name: "parakeet.cpp (derived code in NeMo-Speech.cpp)", version: "1675ee5b", license: "MIT", url: "https://github.com/jason-ni/parakeet.cpp",
      texts: [["MIT", "MIT License", () => `${parakeetNotice()}\n${nemoMit()}`]] },
    // The optional Whisper final pass is desktop-only: static whisper.cpp with the ggml it
    // bundles (one MIT LICENSE covers both).
    { name: "whisper.cpp and its bundled ggml (static)", version: "v1.9.4", license: "MIT", url: "https://github.com/ggml-org/whisper.cpp", desktopOnly: true,
      texts: [["MIT", "MIT License (whisper.cpp, ggml)", () => read(`${WHISPER}/LICENSE`)]] },
    { name: "SentencePiece (static)", version: "17d7580d", license: "Apache-2.0", url: sp, texts: [["Apache-2.0", "Apache License 2.0", () => read(`${VENDORED}/sentencepiece.LICENSE.txt`)]] },
    { name: "Abseil (in SentencePiece)", version: "bundled with SentencePiece", license: "Apache-2.0", url: "https://github.com/abseil/abseil-cpp", texts: [["Apache-2.0", "Apache License 2.0", () => read(`${VENDORED}/abseil.LICENSE.txt`)]] },
    { name: "protobuf-lite (in SentencePiece)", version: "bundled with SentencePiece", license: "BSD-3-Clause", url: "https://github.com/protocolbuffers/protobuf", texts: [["BSD-3-Clause", "BSD 3-Clause (Google)", () => read(`${VENDORED}/protobuf-lite.LICENSE.txt`)]] },
    { name: "darts-clone (in SentencePiece)", version: "bundled with SentencePiece", license: "BSD-2-Clause", url: "https://github.com/s-yata/darts-clone", texts: [["BSD-2-Clause", "BSD 2-Clause (Susumu Yata)", () => read(`${VENDORED}/darts-clone.LICENSE.txt`)]] },
    { name: "esaxx (in SentencePiece)", version: "bundled with SentencePiece", license: "MIT", url: "https://github.com/hillbig/esaxx", texts: [["MIT", "MIT License (esaxx)", () => read(`${VENDORED}/esaxx.LICENSE.txt`)]] },
    { name: "libopus (via opusic-sys)", version: "1.6.1", license: "BSD-3-Clause", url: "https://opus-codec.org", texts: [["BSD-3-Clause", "BSD 3-Clause (Xiph.Org, Opus)", () => read(`${VENDORED}/opus.COPYING.txt`)]] },
    { name: "SQLCipher (via libsqlite3-sys; SQLite inside is public domain)", version: "4.14.0", license: "BSD-3-Clause", url: "https://www.zetetic.net/sqlcipher/", texts: [["BSD-3-Clause", "BSD 3-Clause (Zetetic LLC)", () => read(`${VENDORED}/sqlcipher.LICENSE.txt`)]] },
    // OpenSSL is SQLCipher's crypto provider on non-Apple targets only.
    { name: "OpenSSL 3 (SQLCipher crypto, Windows)", version: "3.6", license: "Apache-2.0", url: "https://www.openssl.org", desktopOnly: true, texts: [["Apache-2.0", "Apache License 2.0", () => read(`${VENDORED}/openssl.LICENSE.txt`)]] },
  ];
  return all
    .filter((c) => !(c.desktopOnly && app === "mobile"))
    .map((c) => ({
      name: c.name,
      version: c.version,
      license: c.license,
      url: c.url,
      licenseKeys: c.texts.map(([id, name, text]) => addVerbatim(id, name, text())),
    }));
}

// ---- 3. Models (registry.toml: [[model]] tables of scalar keys) ----
function modelsSection() {
  const models = [];
  for (const line of read("crates/ghi-models/registry.toml").split("\n")) {
    if (line.trim() === "[[model]]") models.push({});
    const m = line.match(/^(\w+)\s*=\s*"?([^"]*)"?\s*$/);
    if (m && models.length) models.at(-1)[m[1]] = m[2];
  }
  // OpenMDW-1.1 has no text in the repo (third_party/NATIVE_NOTICES.md only names it),
  // so the model card url is the reference. Others reuse a stored text.
  // A model with its own text in third_party/licenses/<id>.LICENSE.txt (its real
  // copyright holder) uses that; the others reuse a stored text for the licence id.
  const own = (m) => {
    const f = `${VENDORED}/${m.id}.LICENSE.txt`;
    return existsSync(f) ? [addVerbatim(m.license, `${m.license} (${m.repo})`, read(f))] : null;
  };
  const stored = (id) => Object.entries(licenses).filter(([, l]) => l.id === id).map(([k]) => k).sort();
  // The phone ships the speech models only (asr, diarization, voice).
  return models
    .filter((m) => app === "desktop" || ["asr", "diarization", "voice"].includes(m.role))
    .map((m) => {
      const url = `https://huggingface.co/${m.repo}`;
      return {
        id: m.id,
        name: m.repo,
        license: m.license,
        url,
        licenseKeys: own(m) ?? stored(m.license).slice(0, 1),
      };
    })
    .sort((a, b) => (a.id < b.id ? -1 : 1));
}

// ---- 4. Bundled assets (packages/ui/THIRD_PARTY_NOTICES.md) ----
function assetsSection() {
  const fontPkg = {
    "Be Vietnam Pro": "@fontsource/be-vietnam-pro",
    "Source Serif 4": "@fontsource-variable/source-serif-4",
    "Geist Mono": "@fontsource-variable/geist-mono",
  };
  const assets = [];
  let section = "";
  let cur = null;
  for (const line of read("packages/ui/THIRD_PARTY_NOTICES.md").split("\n")) {
    const h = line.match(/^## (.+)/);
    if (h) {
      section = h[1];
      cur = null;
      continue;
    }
    const item = line.match(/^(?:- )?\*\*(.+?)\*\*(.*)/);
    if (section && item) {
      cur = { name: item[1], category: section.toLowerCase(), notice: item[2].replace(/^,\s*/, "") };
      assets.push(cur);
    } else if (cur && line.trim() && !line.startsWith("#")) {
      cur.notice += `\n${line.replace(/^>\s?/, "").replace(/^  /, "")}`;
    }
  }
  const stripped = (s) => s.replace(/\s*\(<[^>]*>\)/g, "").replace(/<(https?:[^>]+)>/g, "$1");
  // The phone uses Material Symbols (packages/ui/THIRD_PARTY_NOTICES.md): no Fluent icons.
  return assets
    .filter((a) => app === "desktop" || !/Fluent/i.test(a.name))
    .map((a) => {
      const notice = stripped(a.notice).replace(/\n{2,}/g, "\n").trim();
      const license = /Open Font License/.test(`${a.notice}`) || fontPkg[a.name] || /Fonts/i.test(a.category)
        ? "OFL-1.1"
        : /Apache License/.test(notice) ? "Apache-2.0" : /MIT License/.test(notice) ? "MIT" : "unknown";
      const entry = { name: a.name, license, notice, licenseKeys: [] };
      if (license === "OFL-1.1" && fontPkg[a.name]) {
        const f = join(root, "packages/ui/node_modules", fontPkg[a.name], "LICENSE");
        if (existsSync(f)) entry.licenseKeys = [addText("OFL-1.1", "SIL Open Font License 1.1", readFileSync(f, "utf8")).key];
      } else if (license !== "unknown") {
        // Use the canonical stored text (Apache/MIT) from the Rust section.
        entry.licenseKeys = Object.entries(licenses).filter(([, l]) => l.id === license).map(([k]) => k).sort().slice(0, 1);
      }
      return entry;
    })
    .sort((a, b) => (a.name < b.name ? -1 : 1));
}

const rust = rustSection();
const js = jsSection();
const models = modelsSection();
const assets = assetsSection();
const native = nativeSection();

// Drop unreferenced texts (the JS pass can register variants nothing points to).
// The desktop About lists "assets" (no UI change needed): native components join
// them there. The phone's About has its own "native" group.
if (app === "desktop") {
  for (const n of native) assets.push({ name: n.name, license: n.license, notice: `${n.version} · ${n.url}`, licenseKeys: n.licenseKeys });
  assets.sort((a, b) => (a.name < b.name ? -1 : 1));
}
const used = new Set([...rust, ...js, ...models, ...assets, ...(app === "mobile" ? native : [])].flatMap((e) => e.licenseKeys));
const sorted = Object.fromEntries(
  Object.keys(licenses).filter((k) => used.has(k)).sort().map((k) => [k, licenses[k]]),
);

writeFileSync(out, `${JSON.stringify({ licenses: sorted, rust, js, models, assets, ...(app === "mobile" ? { native } : {}) }, null, 1)}\n`);
console.log(
  `licenses.json: ${Object.keys(sorted).length} texts, ${rust.length} crates, ${js.length} js, ` +
    `${models.length} models, ${assets.length} assets`,
);
