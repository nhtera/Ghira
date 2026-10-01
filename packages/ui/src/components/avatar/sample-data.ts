// SPDX-License-Identifier: Apache-2.0
// Story data from mocks/sample-meeting.json: the speakers of that meeting and
// helpers to read its transcript and notes in either language.
import meeting from "../../../mocks/sample-meeting.json";

export type SampleSpeaker = { id: number; label: string; colorSlot: number; isMe?: boolean };

/** Order of arrival; the mock's `s` indexes into this (design: Me s1, Linh s2, Minh s4, Sarah s8). */
export const SAMPLE_SPEAKERS: SampleSpeaker[] = [
  { id: 0, label: "Me", colorSlot: 1, isMe: true },
  { id: 1, label: "Linh", colorSlot: 2 },
  { id: 2, label: "Minh", colorSlot: 4 },
  { id: 3, label: "Sarah", colorSlot: 8 },
];

export type SampleLine = { startMs: number; speaker: SampleSpeaker; text: string; low?: string };

export function sampleLine(i: number): SampleLine {
  const l = meeting.transcript[i] as { s: number; x: string; low?: string };
  return { startMs: meeting.lineStartSeconds[i]! * 1000, speaker: SAMPLE_SPEAKERS[l.s]!, text: l.x, low: l.low };
}

export const SAMPLE_DURATION_MS = meeting.durationSeconds * 1000;

export const sampleNotes = meeting.notes;

/** The mock's `{en, vi}` text in the active language. */
export function pick(o: { en: string; vi: string }, lang: string): string {
  return lang === "vi" ? o.vi : o.en;
}

/** Start time (ms) of the transcript line a note cites. */
export function citedMs(lineIndex: number): number {
  return meeting.lineStartSeconds[lineIndex]! * 1000;
}
