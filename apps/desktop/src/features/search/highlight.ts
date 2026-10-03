// SPDX-License-Identifier: Apache-2.0
// Snippet highlights: the store sends `[start, end)` ranges in UTF-16 code
// units of the snippet. Rendered as <mark> text nodes only (RT-6).
export type Part = { text: string; mark: boolean };

export function splitHighlights(text: string, ranges: readonly (readonly [number, number])[]): Part[] {
  // Clip, drop empties, sort and merge overlaps so a bad range can't duplicate text.
  const clean = ranges
    .map(([a, b]) => [Math.max(0, Math.min(a, text.length)), Math.max(0, Math.min(b, text.length))] as const)
    .filter(([a, b]) => b > a)
    .sort((x, y) => x[0] - y[0]);
  const merged: [number, number][] = [];
  for (const [a, b] of clean) {
    const last = merged[merged.length - 1];
    if (last && a <= last[1]) last[1] = Math.max(last[1], b);
    else merged.push([a, b]);
  }
  const parts: Part[] = [];
  let at = 0;
  for (const [a, b] of merged) {
    if (a > at) parts.push({ text: text.slice(at, a), mark: false });
    parts.push({ text: text.slice(a, b), mark: true });
    at = b;
  }
  if (at < text.length) parts.push({ text: text.slice(at), mark: false });
  return parts;
}

/** Lowercase, accents and đ folded away: what the store matches on. */
const fold = (c: string) => c.normalize("NFD").replace(/\p{M}/gu, "").replace(/đ/gi, "d").toLowerCase();

/**
 * Ranges of `text` that match any word of `query`, accents ignored (the same
 * folding the search uses), for text the store sent no highlights for (titles).
 */
export function queryRanges(text: string, query: string): [number, number][] {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (words.length === 0) return [];
  // Fold per UTF-16 unit so folded[i] lines up with text[i] (a folded unit may vanish: keep it empty).
  const units = Array.from(text, (c) => fold(c));
  let folded = "";
  const at: number[] = []; // folded index -> text index
  let i = 0;
  for (const u of units) {
    for (let k = 0; k < u.length; k++) at.push(i);
    folded += u;
    i += text.codePointAt(i)! > 0xffff ? 2 : 1;
  }
  const out: [number, number][] = [];
  for (const w of words) {
    for (let from = folded.indexOf(w); from >= 0; from = folded.indexOf(w, from + w.length)) {
      const start = at[from]!;
      const lastFolded = at[from + w.length - 1]!;
      const end = lastFolded + (text.codePointAt(lastFolded)! > 0xffff ? 2 : 1);
      out.push([start, end]);
    }
  }
  return out;
}
