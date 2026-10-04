// SPDX-License-Identifier: Apache-2.0
// Pure rules of the speaker side panel: who can be merged into, which lines a
// split would move, and the sentence for each error code of the core.
import type { MeetingSpeaker, SegmentView } from "../../bindings";

/** The codes `speakers_cmd` answers with; each has a sentence under `speakerPanel.errors`. */
export const ERROR_CODES = ["liveMeeting", "notASpeaker", "sameSpeaker", "farSide", "nothingToSplit", "wholeSpeaker", "isMe", "storage"] as const;
export type PanelErrorCode = (typeof ERROR_CODES)[number];

export const isPanelError = (e: string): e is PanelErrorCode => (ERROR_CODES as readonly string[]).includes(e);

/** Everyone the speaker can be merged into: another person, not themselves. */
export const mergeTargets = (speakers: readonly MeetingSpeaker[], from: string): MeetingSpeaker[] => speakers.filter((s) => s.gid !== from && !s.notPerson);

/** The speaker's lines in the order they were said. */
export const ownLines = (segments: readonly SegmentView[], speaker: string): SegmentView[] =>
  segments.filter((s) => s.speakerGid === speaker).sort((a, b) => (a.t0Ms ?? 0) - (b.t0Ms ?? 0));

/** The lines "from this line on" would move: the line itself and every later line of the speaker. */
export const linesFrom = (own: readonly SegmentView[], from: string | null): SegmentView[] => {
  const i = own.findIndex((s) => s.gid === from);
  return i < 0 ? [] : own.slice(i);
};

export type SplitChoice = { mode: "from"; from: string } | { mode: "lines"; picked: readonly string[] };

/** The command's two arguments: exactly one of the line list and the starting line is set. */
export const splitArgs = (c: SplitChoice): { segmentGids: string[]; fromSegment: string | null } =>
  c.mode === "from" ? { segmentGids: [], fromSegment: c.from } : { segmentGids: [...c.picked], fromSegment: null };

/** How many lines the choice moves (0: nothing to do yet). */
export const splitCount = (own: readonly SegmentView[], c: SplitChoice): number => (c.mode === "from" ? linesFrom(own, c.from).length : c.picked.length);

/** A split needs some of the speaker's lines but not all of them. */
export const splitAllowed = (own: readonly SegmentView[], c: SplitChoice): boolean => {
  const n = splitCount(own, c);
  return n > 0 && n < own.length;
};
