// SPDX-License-Identifier: Apache-2.0
// What the phone app ships, by name and license. src/generated/licenses.json is
// generated (`pnpm gen:licenses:mobile`: iOS crates, bundled npm packages, the
// pinned models and the bundled fonts/icons); do not edit it by hand.
import { useEffect, useState } from "react";

type Text = { id: string; name: string; text: string };
type Pkg = { name: string; version: string; license: string; licenseKeys: string[]; copyright?: string[] };
type Model = { id: string; name: string; license: string; url: string; licenseKeys: string[] };
type Asset = { name: string; license: string; notice: string; licenseKeys: string[] };
type Native = { name: string; version: string; license: string; url: string; licenseKeys: string[] };
export type Generated = { licenses: Record<string, Text>; rust: Pkg[]; js: Pkg[]; models: Model[]; assets: Asset[]; native: Native[] };

export type LicenseGroup = "models" | "assets" | "native" | "rust" | "js";
export type LicenseRow = {
  id: string;
  group: LicenseGroup;
  name: string;
  version: string;
  license: string;
  /** Texts de-duplicated and joined; empty when none is bundled. */
  text: string;
  copyright: string[];
};

export const GROUPS: LicenseGroup[] = ["models", "assets", "native", "rust", "js"];

export function licenseRows(data: Generated): LicenseRow[] {
  const text = (keys: string[]) =>
    [...new Set(keys)]
      .map((k) => data.licenses[k]?.text ?? "")
      .filter(Boolean)
      .join("\n\n---\n\n");
  const pkg = (group: "rust" | "js") => (p: Pkg): LicenseRow => ({
    id: `${group}:${p.name}@${p.version}`,
    group,
    name: p.name,
    version: p.version,
    license: p.license,
    text: text(p.licenseKeys),
    copyright: p.copyright ?? [],
  });
  return [
    ...data.models.map((m): LicenseRow => ({ id: `model:${m.id}`, group: "models", name: m.name, version: "", license: m.license, text: text(m.licenseKeys), copyright: [m.url] })),
    ...data.assets.map((a): LicenseRow => ({ id: `asset:${a.name}`, group: "assets", name: a.name, version: "", license: a.license, text: text(a.licenseKeys), copyright: a.notice ? [a.notice] : [] })),
    ...data.native.map((n): LicenseRow => ({ id: `native:${n.name}`, group: "native", name: n.name, version: n.version, license: n.license, text: text(n.licenseKeys), copyright: [n.url] })),
    ...data.rust.map(pkg("rust")),
    ...data.js.map(pkg("js")),
  ].map((r, i) => ({ ...r, id: `${r.id}#${i}` })); // unique even if a crate appears twice
}

/** Every word of the query must appear in the name, version or license. */
export function filterLicenses(rows: LicenseRow[], query: string): LicenseRow[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return rows;
  return rows.filter((r) => {
    const hay = `${r.name} ${r.version} ${r.license}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}

/** Loads the generated data as its own chunk (about 350 kB). */
export function useLicenseRows(): LicenseRow[] | null {
  const [rows, setRows] = useState<LicenseRow[] | null>(null);
  useEffect(() => {
    let alive = true;
    void import("../../generated/licenses.json").then((m) => {
      if (alive) setRows(licenseRows(m.default as Generated));
    });
    return () => {
      alive = false;
    };
  }, []);
  return rows;
}

/** The value after it has been still for `ms` (for what a screen reader announces). */
export function useDebounced<T>(value: T, ms = 500): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}
