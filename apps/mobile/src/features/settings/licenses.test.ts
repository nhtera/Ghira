// SPDX-License-Identifier: Apache-2.0
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import data from "../../generated/licenses.json";
import { filterLicenses, licenseRows, type Generated } from "./licenses";

const rows = licenseRows(data as Generated);

/** about.toml's `accepted` list: the licenses cargo-about (and deny.toml) allow. */
const accepted = new Set([...readFileSync(join(process.cwd(), "../../about.toml"), "utf8").match(/accepted = \[([^\]]*)\]/)![1]!.matchAll(/"([^"]+)"/g)].map((m) => m[1]!));

/** An SPDX expression is allowed when OR has an allowed side and AND has all sides allowed. */
function allowed(expr: string): boolean {
  const e = expr.replace(/\s+/g, " ").trim();
  if (accepted.has(e)) return true; // includes "Apache-2.0 WITH LLVM-exception"
  const depth0 = (op: string) => {
    let d = 0;
    const parts: string[] = [];
    let cur = "";
    for (const tok of e.split(/(\(|\)| )/).filter((t) => t !== "")) {
      if (tok === "(") d++;
      if (tok === ")") d--;
      if (d === 0 && tok === op) {
        parts.push(cur.trim());
        cur = "";
      } else cur += tok;
    }
    parts.push(cur.trim());
    return parts;
  };
  const or = depth0("OR");
  if (or.length > 1) return or.some(allowed);
  const and = depth0("AND");
  if (and.length > 1) return and.every(allowed);
  return e.startsWith("(") && e.endsWith(")") ? allowed(e.slice(1, -1)) : false;
}

describe("licenses", () => {
  it("lists the models, bundled assets, crates and npm packages", () => {
    for (const g of ["models", "assets", "rust", "js"] as const) expect(rows.some((r) => r.group === g)).toBe(true);
    expect(rows.find((r) => r.name === "nvidia/Nemotron-3-Diarization")?.license).toBe("OpenMDW-1.1");
  });
  it("has no desktop-only crates and no GPL", () => {
    expect(rows.some((r) => /^(tauri-plugin-(updater|dialog)|rfd)$/.test(r.name))).toBe(false);
    expect(rows.some((r) => /\b(A?GPL|SSPL)/.test(r.license))).toBe(false);
  });
  it("filters on every word", () => {
    const hits = filterLicenses(rows, "react mit");
    expect(hits.length).toBeGreaterThan(0);
    expect(hits.every((r) => /react/i.test(r.name) && /mit/i.test(r.license))).toBe(true);
    expect(filterLicenses(rows, "")).toHaveLength(rows.length);
  });
  it("ships only licenses cargo-about accepts (no LGPL slips in)", () => {
    const bad = rows.filter((r) => ["rust", "native"].includes(r.group) && !allowed(r.license)).map((r) => `${r.name}: ${r.license}`);
    expect(bad).toEqual([]);
    expect(rows.filter((r) => r.group === "js" && /\b(A?GPL|LGPL|SSPL)/.test(r.license))).toEqual([]);
  });
  it("lists the native libraries with their license texts, NeMo-Speech.cpp's NOTICE included", () => {
    const native = rows.filter((r) => r.group === "native");
    const nemo = native.find((r) => r.name.startsWith("NeMo-Speech.cpp"));
    expect(nemo?.text).toContain("Apache License");
    expect(nemo?.text).toContain("Version 2.0");
    expect(nemo?.text).toContain("NVIDIA CORPORATION"); // the NOTICE file (Apache-2.0 4(d))
    for (const n of ["ggml", "parakeet.cpp", "SentencePiece", "Abseil", "protobuf-lite", "darts-clone", "libopus"]) {
      expect(native.some((r) => r.name.startsWith(n) && r.text.length > 200), n).toBe(true);
    }
    const sqlcipher = native.find((r) => r.name.startsWith("SQLCipher"));
    expect(sqlcipher?.license).toBe("BSD-3-Clause");
    expect(sqlcipher?.text).toContain("Zetetic LLC");
  });
  it("lists only the models the phone ships, and no Fluent icons", () => {
    expect(rows.filter((r) => r.group === "models").map((r) => r.name).sort()).toEqual([
      "csukuangfj/speaker-embedding-models",
      "nvidia/Nemotron-3-Diarization",
      "nvidia/nemotron-3.5-asr-streaming-0.6b",
    ]);
    expect(rows.some((r) => /Fluent/i.test(r.name))).toBe(false);
  });
  it("gives every row a unique id", () => {
    expect(new Set(rows.map((r) => r.id)).size).toBe(rows.length);
  });
});
