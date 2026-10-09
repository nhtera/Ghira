// SPDX-License-Identifier: Apache-2.0
// Pure helpers of the transcript tab (kept apart so they test without a DOM):
// VN-folded find, speaker grouping, karaoke word lookup.
import type { MarkView, SegmentView, TopicView } from "../../bindings";

/** Words below this confidence are underlined (brief D6). */
export const LOW_CONFIDENCE = 0.5;
/** A speaker's paragraph is cut after this many lines so rows stay a readable height. */
export const MAX_LINES_PER_GROUP = 8;

const MARKS = /[̀-ͯ]/g;

/**
 * Lowercase, accent-free (NFD marks stripped, đ → d), one UTF-16 unit per
 * unit, so an index in the folded text is the same index in the original.
 * (The core stores NFC text; a stray combining mark is kept as it is.)
 */
export function fold(s: string): string {
  let out = "";
  for (let i = 0; i < s.length; i++) {
    const c = s.charAt(i);
    const f = c.normalize("NFD").replace(MARKS, "").replace(/[đĐ]/, "d").toLowerCase();
    out += f.length === 1 ? f : c.toLowerCase().length === 1 ? c.toLowerCase() : c;
  }
  return out;
}

/** Folds a typed query (any normalization form) the same way, trimmed and with runs of spaces collapsed. */
export const foldQuery = (q: string) => fold(q.normalize("NFC").trim().replace(/\s+/g, " "));

/** Every non-overlapping occurrence of `query` in `text`, as [start, end) in the original text. */
export function findRanges(text: string, query: string): [number, number][] {
  return rangesIn(fold(text), foldQuery(query));
}

function rangesIn(f: string, q: string): [number, number][] {
  if (!q) return [];
  const out: [number, number][] = [];
  for (let i = f.indexOf(q); i >= 0; i = f.indexOf(q, i + q.length)) out.push([i, i + q.length]);
  return out;
}

export type Match = { seg: number; start: number; end: number };

// Folding every line again on each keystroke is wasted work: keep it per line object (a refetch gives new objects).
const folded = new WeakMap<object, string>();
const foldedText = (s: Pick<SegmentView, "text">) => {
  let f = folded.get(s);
  if (f === undefined) folded.set(s, (f = fold(s.text)));
  return f;
};

/** All matches across the segments, in transcript order. */
export function findMatches(segments: readonly Pick<SegmentView, "text">[], query: string): Match[] {
  const q = foldQuery(query);
  if (!q) return [];
  return segments.flatMap((s, seg) => rangesIn(foldedText(s), q).map(([start, end]) => ({ seg, start, end })));
}

/** One whitespace-split word of a line with its offset in the text (the words[] of the core align 1:1 with this). */
export type LineWord = { text: string; offset: number; t0Ms: number | null; low: boolean };

export function lineWords(seg: SegmentView): LineWord[] {
  const parts = seg.text.split(" ");
  const aligned = seg.words.length === parts.length;
  let offset = 0;
  return parts.map((text, i) => {
    const w = aligned ? seg.words[i] : undefined;
    const word = { text, offset, t0Ms: w?.t0Ms ?? null, low: w?.confidence != null && w.confidence < LOW_CONFIDENCE };
    offset += text.length + 1;
    return word;
  });
}

/** The word being spoken at `ms`: the last whose start has passed (-1 before the first, or without timings). */
export function wordIndexAt(words: readonly Pick<LineWord, "t0Ms">[], ms: number): number {
  let at = -1;
  for (let i = 0; i < words.length; i++) {
    const t0 = words[i]!.t0Ms;
    if (t0 == null) return -1;
    if (t0 <= ms) at = i;
    else break;
  }
  return at;
}

/** The segment being played at `ms` (last starting at or before it), if `ms` is not far past its end. */
export function segmentAt(segments: readonly Pick<SegmentView, "t0Ms" | "t1Ms">[], ms: number, graceMs = 1500): number {
  let lo = 0;
  let hi = segments.length - 1;
  let at = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if ((segments[mid]!.t0Ms ?? 0) <= ms) {
      at = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  if (at < 0) return -1;
  const end = segments[at]!.t1Ms ?? segments[at]!.t0Ms ?? 0;
  return ms <= end + graceMs ? at : -1;
}

export type GroupData = { kind: "group"; key: string; speakerGid: string | null; first: number; segs: SegmentView[] };
export type StackRow = { kind: "stack"; key: string; first: number; groups: GroupData[] };
export type Row = { kind: "topic"; key: string; title: string; tMs: number } | GroupData | StackRow;

export type StackSpan = { speaker: string | number | null; t0Ms: number | null; t1Ms: number | null; overlap: boolean };

/**
 * Runs of consecutive lines to draw as one stack, as `[start, end)` index
 * ranges. Only lines the core flagged as talked over (`overlap`) start one:
 * a flagged line takes in the lines next to it that overlap it in time (a
 * "mm-hm" inside a long turn brings in just that turn), and flagged lines
 * that share a neighbour join into one run. A run holds at most
 * `MAX_LINES_PER_GROUP` lines (a longer one is cut), needs at least two
 * speakers, and never crosses an index in `cuts` (a topic header goes there).
 * Ordinary turn-taking, even with a hair of overlap, is not stacked. Purely
 * presentational: the lines keep their identity and order.
 */
export function stackRuns(spans: readonly StackSpan[], cuts: readonly number[] = []): [number, number][] {
  const cut = new Set(cuts);
  const t0 = (x: StackSpan) => x.t0Ms ?? 0;
  const t1 = (x: StackSpan) => Math.max(x.t1Ms ?? t0(x), t0(x));
  const touches = (a: StackSpan, b: StackSpan) => t0(a) < t1(b) && t0(b) < t1(a);
  // The reach of each flagged line, merged where they meet.
  const ranges: [number, number][] = [];
  spans.forEach((x, i) => {
    if (!x.overlap) return;
    let lo = i;
    while (lo > 0 && !cut.has(lo) && touches(spans[lo - 1]!, x)) lo--;
    let hi = i + 1;
    while (hi < spans.length && !cut.has(hi) && touches(spans[hi]!, x)) hi++;
    const last = ranges.at(-1);
    if (last && lo < last[1]) last[1] = Math.max(last[1], hi);
    else ranges.push([lo, hi]);
  });
  const runs: [number, number][] = [];
  for (const [lo, hi] of ranges) {
    for (let a = lo; a < hi; a += MAX_LINES_PER_GROUP) {
      const b = Math.min(hi, a + MAX_LINES_PER_GROUP);
      if (b - a > 1 && new Set(spans.slice(a, b).map((x) => x.speaker)).size > 1) runs.push([a, b]);
    }
  }
  return runs;
}

/**
 * Rows of the transcript: speaker paragraphs (consecutive lines of one
 * speaker, cut at a topic header or after MAX_LINES_PER_GROUP lines) with the
 * topic headers between them. `first` is the index of the group's first line
 * in the segment array. Lines the core flagged as talked over, with the lines
 * they overlap in time, form one `stack` row of paragraphs ([`stackRuns`]),
 * drawn inside one bracket.
 */
export function buildRows(segments: readonly SegmentView[], topics: readonly TopicView[]): Row[] {
  const rows: Row[] = [];
  const heads = [...topics].filter((t) => t.tMs != null).sort((a, b) => a.tMs! - b.tMs!);
  // The line each topic header sits before: runs may not cross it.
  const cuts = heads.map((h) => segments.findIndex((s) => (s.t0Ms ?? 0) >= h.tMs!)).filter((i) => i > 0);
  const runAt = new Map<number, number>();
  for (const [a, b] of stackRuns(segments.map((s) => ({ speaker: s.speakerGid, t0Ms: s.t0Ms, t1Ms: s.t1Ms, overlap: s.overlap })), cuts)) runAt.set(a, b);
  let next = 0;
  let cur: GroupData | null = null;
  let skipTo = 0;
  segments.forEach((seg, i) => {
    const t0 = seg.t0Ms ?? 0;
    // A topic goes before the first line that starts at or after it.
    while (next < heads.length && heads[next]!.tMs! <= t0) {
      const h = heads[next]!;
      rows.push({ kind: "topic", key: `topic-${next}`, title: h.title, tMs: h.tMs! });
      next++;
      cur = null;
    }
    if (i < skipTo) return;
    const runEnd = runAt.get(i);
    if (runEnd !== undefined) {
      const groups: GroupData[] = [];
      let g: GroupData | null = null;
      for (let k = i; k < runEnd; k++) {
        const s = segments[k]!;
        if (!g || g.speakerGid !== s.speakerGid) {
          g = { kind: "group", key: `g-${s.gid}`, speakerGid: s.speakerGid, first: k, segs: [] };
          groups.push(g);
        }
        g.segs.push(s);
      }
      rows.push({ kind: "stack", key: `s-${seg.gid}`, first: i, groups });
      skipTo = runEnd;
      cur = null;
      return;
    }
    if (!cur || cur.speakerGid !== seg.speakerGid || cur.segs.length >= MAX_LINES_PER_GROUP) {
      cur = { kind: "group", key: `g-${seg.gid}`, speakerGid: seg.speakerGid, first: i, segs: [] };
      rows.push(cur);
    }
    cur.segs.push(seg);
  });
  return rows;
}

/** The index of the segment's row, per segment, for scrolling to it. */
export function rowIndexBySegment(rows: readonly Row[], count: number): number[] {
  const by = new Array<number>(count).fill(0);
  rows.forEach((r, ri) => {
    const groups = r.kind === "group" ? [r] : r.kind === "stack" ? r.groups : [];
    for (const g of groups) g.segs.forEach((_, k) => (by[g.first + k] = ri));
  });
  return by;
}

/** The marks that fall in a line (a mark belongs to the last line starting at or before it). */
export function marksOf(segments: readonly SegmentView[], marks: readonly MarkView[]): Map<number, MarkView[]> {
  const by = new Map<number, MarkView[]>();
  for (const m of marks) {
    if (m.tMs == null) continue;
    let at = -1;
    segments.forEach((s, i) => {
      if ((s.t0Ms ?? 0) <= m.tMs!) at = i;
    });
    if (at >= 0) by.set(at, [...(by.get(at) ?? []), m]);
  }
  return by;
}

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
 * Structural types only: no app bindings (it moves to @ghi/ui later).
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
