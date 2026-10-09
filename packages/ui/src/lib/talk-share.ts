// SPDX-License-Identifier: Apache-2.0
// Who talked how much: pure, React-free and binding-free (both apps use it).

export type ShareInput = { speakerGid: string | null; t0Ms: number | null; t1Ms: number | null };
export type ShareEntry = {
  /** `null`: lines with no (known) speaker. */
  gid: string | null;
  talkMs: number;
  /** Whole percent; the entries always add up to exactly 100. */
  pct: number;
  turns: number;
};

/**
 * Who talked how much: Σ (t1 - t0) per speaker over the lines, biggest first.
 * `speakers[].lines` is the turn count shown beside the share; a line whose
 * speaker is missing from `speakers` counts as unassigned. Percentages are
 * rounded by largest remainder so they sum to 100. Empty without any talk time.
 * Structural types only: no React, no app bindings (shared by both apps).
 */
export function talkShare(segments: readonly ShareInput[], speakers: readonly { gid: string; lines: number }[]): ShareEntry[] {
  const known = new Map(speakers.map((s) => [s.gid, s.lines]));
  const ms = new Map<string | null, number>();
  let unassignedLines = 0;
  for (const s of segments) {
    const gid = s.speakerGid != null && known.has(s.speakerGid) ? s.speakerGid : null;
    if (gid === null) unassignedLines++;
    const d = Math.max(0, (s.t1Ms ?? s.t0Ms ?? 0) - (s.t0Ms ?? 0));
    ms.set(gid, (ms.get(gid) ?? 0) + d);
  }
  const total = [...ms.values()].reduce((a, b) => a + b, 0);
  if (total <= 0) return [];
  const raw = [...ms].filter(([, d]) => d > 0).map(([gid, talkMs]) => ({ gid, talkMs, exact: (talkMs / total) * 100 }));
  const pcts = raw.map((r) => Math.floor(r.exact));
  let left = 100 - pcts.reduce((a, b) => a + b, 0);
  [...raw.keys()]
    .sort((a, b) => raw[b]!.exact - pcts[b]! - (raw[a]!.exact - pcts[a]!) || raw[b]!.talkMs - raw[a]!.talkMs)
    .forEach((i) => {
      if (left-- > 0) pcts[i]!++;
    });
  return raw
    .map((r, i) => ({ gid: r.gid, talkMs: r.talkMs, pct: pcts[i]!, turns: r.gid === null ? unassignedLines : (known.get(r.gid) ?? 0) }))
    .sort((a, b) => b.talkMs - a.talkMs);
}
