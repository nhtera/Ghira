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

export type Row =
  | { kind: "topic"; key: string; title: string; tMs: number }
  | { kind: "group"; key: string; speakerGid: string | null; first: number; segs: SegmentView[] };

/**
 * Rows of the transcript: speaker paragraphs (consecutive lines of one
 * speaker, cut at a topic header or after MAX_LINES_PER_GROUP lines) with the
 * topic headers between them. `first` is the index of the group's first line
 * in the segment array.
 */
export function buildRows(segments: readonly SegmentView[], topics: readonly TopicView[]): Row[] {
  const rows: Row[] = [];
  const heads = [...topics].filter((t) => t.tMs != null).sort((a, b) => a.tMs! - b.tMs!);
  let next = 0;
  let cur: Extract<Row, { kind: "group" }> | null = null;
  segments.forEach((seg, i) => {
    const t0 = seg.t0Ms ?? 0;
    // A topic goes before the first line that starts at or after it.
    while (next < heads.length && heads[next]!.tMs! <= t0) {
      const h = heads[next]!;
      rows.push({ kind: "topic", key: `topic-${next}`, title: h.title, tMs: h.tMs! });
      next++;
      cur = null;
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
    if (r.kind === "group") r.segs.forEach((_, k) => (by[r.first + k] = ri));
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
