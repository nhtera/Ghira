// SPDX-License-Identifier: Apache-2.0
// Pure helpers of the live view (kept apart so they test without a DOM).
import type { LineInfo, SpeakerInfo } from "../../bindings";
import { stackRuns } from "../transcript/logic";

/** Distance (px) from the bottom within which the transcript keeps following the newest line. */
export const FOLLOW_SLOP_PX = 48;

export const distanceFromBottom = (el: { scrollTop: number; clientHeight: number; scrollHeight: number }) => el.scrollHeight - el.scrollTop - el.clientHeight;

/** The user is at the bottom (or close): new lines keep scrolling; scrolling up stops it. */
export const isFollowing = (el: Parameters<typeof distanceFromBottom>[0]) => distanceFromBottom(el) <= FOLLOW_SLOP_PX;

/** Seconds without any level before "waiting for audio" shows (brief D4). */
export const NO_AUDIO_MS = 10_000;

/** No levels for a while while recording: the source may be wrong. */
export function waitingForAudio(p: { recording: boolean; asleep: boolean; lastLevelAtMs: number; nowMs: number }): boolean {
  return p.recording && !p.asleep && p.nowMs - p.lastLevelAtMs >= NO_AUDIO_MS;
}

/** Above this the speaker detection is behind enough to suggest Fast mode. */
export const LAG_WARN_S = 3;

/** About 32 kbps Opus: minutes of recording that fit in `bytes` (a rough hint, not a promise). */
export const minutesLeft = (bytes: number) => Math.max(0, Math.floor(bytes / 240_000));

/** "1.5 GB" / "480 MB" with the locale's decimal separator ("1,5 GB" in Vietnamese). */
export { formatBytes } from "@ghi/i18n";

export type Lane = { id: number; label: string; colorSlot: number };
export type Segment = { speaker: number; t0Ms: number; t1Ms: number };

/** Turns closer than this (same speaker) are drawn as one segment: thousands of lines stay cheap. */
const MERGE_GAP_MS = 1500;
/** Speakers beyond this many share the "Others" lane (id 0). */
export const MAX_LANES = 8;

/**
 * The speakers who get their own lane and chip (in arrival order, at most
 * `MAX_LANES`, with a color) and the ones who share Others: those past the
 * limit and any the core already put there.
 */
export function splitOthers(speakers: readonly SpeakerInfo[]): { own: SpeakerInfo[]; others: SpeakerInfo[] } {
  const ordered = [...speakers].filter((s) => !s.notPerson).sort((a, b) => a.id - b.id);
  const own = ordered.filter((s) => !s.others && s.colorSlot > 0).slice(0, MAX_LANES);
  const ids = new Set(own.map((s) => s.id));
  return { own, others: ordered.filter((s) => !ids.has(s.id)) };
}

/**
 * Lane rows and merged segments from live lines. Speakers are in arrival order
 * (id order); the ones past `MAX_LANES`, and any the core already put in
 * Others, fold into one shared lane (`othersLabel`, e.g. "Others · 3").
 */
export function laneModel(speakers: SpeakerInfo[], lines: LineInfo[], labelOf: (s: SpeakerInfo) => string, othersLabel: string): { lanes: Lane[]; segments: Segment[] } {
  const { own, others } = splitOthers(speakers);
  const laneOf = new Map<number, number>(own.map((s) => [s.id, s.id]));
  const lanes: Lane[] = own.map((s) => ({ id: s.id, label: labelOf(s), colorSlot: s.colorSlot }));
  if (others.length > 0) {
    lanes.push({ id: 0, label: othersLabel, colorSlot: 0 });
    for (const s of others) laneOf.set(s.id, 0);
  }
  const segments: Segment[] = [];
  const last = new Map<number, Segment>();
  for (const l of lines) {
    if (l.speaker == null || l.t0Ms == null || l.t1Ms == null) continue;
    const lane = laneOf.get(l.speaker);
    if (lane === undefined) continue;
    const prev = last.get(lane);
    if (prev && l.t0Ms - prev.t1Ms <= MERGE_GAP_MS) {
      prev.t1Ms = Math.max(prev.t1Ms, l.t1Ms);
      continue;
    }
    const seg = { speaker: lane, t0Ms: l.t0Ms, t1Ms: l.t1Ms };
    segments.push(seg);
    last.set(lane, seg);
  }
  return { lanes, segments };
}

export type Inline = { text: string; bold?: boolean; italic?: boolean };
export type NoteLineView = { bullet: boolean; parts: Inline[] };

/**
 * Markdown-lite for one notepad line: a leading "- " or "* " is a bullet,
 * **bold** and _italic_ are styled. The result is data; the UI renders text
 * nodes only (RT-6), never HTML.
 */
export function parseNoteLine(text: string): NoteLineView {
  const m = /^\s*[-*]\s+(.*)$/.exec(text);
  const body = m ? m[1] : text;
  const parts: Inline[] = [];
  const re = /\*\*([^*]+)\*\*|_([^_]+)_/g;
  let at = 0;
  for (let hit = re.exec(body); hit; hit = re.exec(body)) {
    if (hit.index > at) parts.push({ text: body.slice(at, hit.index) });
    parts.push(hit[1] !== undefined ? { text: hit[1], bold: true } : { text: hit[2], italic: true });
    at = hit.index + hit[0].length;
  }
  if (at < body.length) parts.push({ text: body.slice(at) });
  return { bullet: Boolean(m), parts };
}

/** What the live transcript draws: one line, or a stack of lines the core flagged as talked over. */
export type LiveBlock = { from: number; to: number; stacked: boolean };

/**
 * The lines as blocks. Lines flagged as talked over, with the neighbours they
 * overlap in time, form one stacked block ([`stackRuns`]); everything else is
 * one block per line. The lines themselves are untouched.
 */
export function liveBlocks(lines: readonly Pick<LineInfo, "speaker" | "t0Ms" | "t1Ms" | "overlap">[]): LiveBlock[] {
  const runs = stackRuns(lines.map((l) => ({ speaker: l.speaker, t0Ms: l.t0Ms, t1Ms: l.t1Ms, overlap: l.overlap })));
  const out: LiveBlock[] = [];
  let r = 0;
  for (let i = 0; i < lines.length; ) {
    const run = runs[r];
    if (run && run[0] === i) {
      out.push({ from: i, to: run[1], stacked: true });
      i = run[1];
      r++;
    } else {
      out.push({ from: i, to: i + 1, stacked: false });
      i++;
    }
  }
  return out;
}
