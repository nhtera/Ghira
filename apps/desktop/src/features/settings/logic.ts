// SPDX-License-Identifier: Apache-2.0
// Pure rules behind the Settings sections (kept apart so they are unit-tested).
import type { Licenses } from "../../generated/licenses";

/** Accent- and case-insensitive key: "Chốt" and "chot" are the same term. */
export const fold = (s: string) =>
  s
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[đĐ]/g, "d")
    .toLowerCase()
    .trim();

export type TermResult = { terms: string[]; status: "added" | "empty" | "duplicate" | "full" };

/** Adds a custom-vocabulary term: trimmed, deduped (accent-insensitive), capped. */
export function addTerm(terms: string[], input: string, max: number): TermResult {
  const term = input.trim().replace(/\s+/g, " ");
  if (!term) return { terms, status: "empty" };
  if (terms.some((t) => fold(t) === fold(term))) return { terms, status: "duplicate" };
  if (terms.length >= max) return { terms, status: "full" };
  return { terms: [...terms, term], status: "added" };
}

/** Replaces term `i`; an empty value removes it, a clash with another term is refused. */
export function editTerm(terms: string[], i: number, input: string): TermResult {
  const term = input.trim().replace(/\s+/g, " ");
  if (!term) return { terms: terms.filter((_, k) => k !== i), status: "empty" };
  if (terms.some((t, k) => k !== i && fold(t) === fold(term))) return { terms, status: "duplicate" };
  return { terms: terms.map((t, k) => (k === i ? term : t)), status: "added" };
}

export const MIN_PASSWORD = 8;

export type PasswordIssue = "short" | "mismatch" | null;

export function passwordIssue(password: string, confirm: string): PasswordIssue {
  if ([...password].length < MIN_PASSWORD) return "short";
  if (password !== confirm) return "mismatch";
  return null;
}

/** 0 weak, 1 fair, 2 good, 3 strong: length plus character variety. */
export function passwordStrength(pw: string): 0 | 1 | 2 | 3 {
  const len = [...pw].length;
  if (len < MIN_PASSWORD) return 0;
  const kinds = [/[a-z]/, /[A-Z]/, /\d/, /[^A-Za-z0-9]/].filter((r) => r.test(pw)).length;
  const score = kinds + (len >= 12 ? 1 : 0) + (len >= 16 ? 1 : 0);
  return score >= 5 ? 3 : score >= 3 ? 2 : 1;
}

/** The word to type before everything is deleted. */
export const deleteWord = (lang: string) => (lang === "vi" ? "XÓA" : "DELETE");
export const deleteWordMatches = (typed: string, word: string) => fold(typed) === fold(word);

export const RETENTION_DAYS = [0, 7, 30, 90, 365] as const;

/** Does moving to `next` delete audio that is kept today? (0 = keep forever) */
export const retentionDeletes = (current: number, next: number) => next > 0 && (current === 0 || next < current);

export type LicenseGroup = "rust" | "js" | "models" | "assets";
export type LicenseRow = {
  id: string;
  group: LicenseGroup;
  name: string;
  version: string;
  license: string;
  /** The license texts, de-duplicated and joined (empty if not bundled). */
  text: string;
  copyright: string[];
  url?: string;
};

export function licenseRows(data: Licenses): LicenseRow[] {
  const text = (keys: string[]) => [...new Set(keys)].map((k) => data.licenses[k]?.text ?? "").filter(Boolean).join("\n\n---\n\n");
  return [
    ...data.rust.map((p) => ({ id: `rust:${p.name}@${p.version}`, group: "rust" as const, name: p.name, version: p.version, license: p.license, text: text(p.licenseKeys), copyright: p.copyright ?? [] })),
    ...data.js.map((p) => ({ id: `js:${p.name}@${p.version}`, group: "js" as const, name: p.name, version: p.version, license: p.license, text: text(p.licenseKeys), copyright: p.copyright ?? [] })),
    ...data.models.map((m) => ({ id: `model:${m.id}`, group: "models" as const, name: m.name, version: "", license: m.license, text: text(m.licenseKeys), copyright: [], url: m.url })),
    ...data.assets.map((a) => ({ id: `asset:${a.name}`, group: "assets" as const, name: a.name, version: "", license: a.license, text: text(a.licenseKeys), copyright: a.notice ? [a.notice] : [] })),
  ];
}

/** Every word of the query must appear in the name, version or license. */
export function filterLicenses(rows: LicenseRow[], query: string): LicenseRow[] {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (!words.length) return rows;
  return rows.filter((r) => {
    const hay = fold(`${r.name} ${r.version} ${r.license}`);
    return words.every((w) => hay.includes(w));
  });
}

/** The pack ids to save after switching `id` on or off (the others stay as they are). */
export function packsAfter(packs: readonly { id: string; enabled: boolean }[], id: string, on: boolean): string[] {
  return packs.filter((p) => (p.id === id ? on : p.enabled)).map((p) => p.id);
}
