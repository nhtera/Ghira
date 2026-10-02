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
