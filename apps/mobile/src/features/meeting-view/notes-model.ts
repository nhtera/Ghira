// SPDX-License-Identifier: Apache-2.0
// Pure mapping from the core's notes and transcript views to what the
// @ghi/ui components draw: provenance, sections, citations, speaker labels.
import type {
  NoteCitation,
  NoteKind,
  TranscriptSpeaker,
  TranscriptWord,
} from "@ghi/ui";
import type {
  Citation,
  MeetingSpeaker,
  NoteBlockView,
  SegmentView,
} from "../../bindings";

/** user / AI / AI-edited; an AI expansion with no text is "not found". */
export function noteKind(
  b: Pick<NoteBlockView, "kind" | "origin" | "text">,
): NoteKind {
  if (b.kind.startsWith("enhanced:") && b.text.trim() === "") return "missing";
  return b.origin === "user"
    ? "user"
    : b.origin === "aiEdited"
      ? "edited"
      : "ai";
}

export type SectionKey =
  "tldr" | "decision" | "question" | "topic" | "quote" | "note" | "other";
export const SECTION_ORDER: SectionKey[] = [
  "tldr",
  "decision",
  "question",
  "topic",
  "quote",
  "note",
  "other",
];

export function sectionOf(kind: string): SectionKey {
  if (kind.startsWith("enhanced:") || kind === "action") return "note";
  return (SECTION_ORDER as string[]).includes(kind)
    ? (kind as SectionKey)
    : "other";
}

/** Blocks grouped by section in reading order; empty sections are left out. */
export function groupBlocks(
  blocks: NoteBlockView[],
): { key: SectionKey; blocks: NoteBlockView[] }[] {
  return SECTION_ORDER.map((key) => ({
    key,
    blocks: blocks.filter((b) => sectionOf(b.kind) === key),
  })).filter((g) => g.blocks.length > 0);
}

/** The chip a citation draws: its time, or its number when it has none. A moment with no words is dashed. */
export function toNoteCitation(
  c: Citation,
  index: number,
  visited: ReadonlySet<string>,
  key: string,
): NoteCitation {
  return {
    ...(c.t0Ms === null ? { index: index + 1 } : { timeMs: c.t0Ms }),
    broken: c.missing,
    visited: visited.has(key),
  };
}

export function speakerOf(
  speakers: MeetingSpeaker[],
  gid: string | null,
): MeetingSpeaker | undefined {
  return gid === null ? undefined : speakers.find((s) => s.gid === gid);
}

/** `numbered` is the localized "Speaker N". Color slot plus initial: never color alone. */
export function transcriptSpeaker(
  s: MeetingSpeaker | undefined,
  numbered: (n: number) => string,
): TranscriptSpeaker | null {
  if (!s) return null;
  return s.name
    ? { label: s.name, colorSlot: s.colorSlot, isMe: s.isMe }
    : {
        label: numbered(s.number),
        colorSlot: s.colorSlot,
        isMe: s.isMe,
        initial: String(s.number),
      };
}

/** Words below this engine confidence are underlined. */
export const LOW_CONFIDENCE = 0.6;

export function segmentWords(s: SegmentView): TranscriptWord[] {
  const texts = s.text.split(/\s+/).filter(Boolean);
  const timed = s.words.length === texts.length;
  return texts.map((text, i) => ({
    text,
    lowConfidence:
      timed &&
      s.words[i].confidence !== null &&
      (s.words[i].confidence ?? 1) < LOW_CONFIDENCE,
  }));
}

/** The segment being played at `timeMs`, or -1. */
export function activeSegment(segments: SegmentView[], timeMs: number): number {
  return segments.findIndex(
    (s) =>
      s.t0Ms !== null &&
      s.t0Ms <= timeMs &&
      (s.t1Ms === null || timeMs < s.t1Ms),
  );
}

/** Karaoke: the word being said, when the engine timed the words. */
export function activeWord(s: SegmentView, timeMs: number): number | undefined {
  const i = s.words.findIndex(
    (w) =>
      w.t0Ms !== null &&
      w.t0Ms <= timeMs &&
      (w.t1Ms === null || timeMs < w.t1Ms),
  );
  return i < 0 ? undefined : i;
}

/** The segment nearest `ms` (search results open here). */
export function segmentNear(segments: SegmentView[], ms: number): number {
  let best = -1;
  let gap = Infinity;
  segments.forEach((s, i) => {
    if (s.t0Ms === null) return;
    const g =
      ms >= s.t0Ms && (s.t1Ms === null || ms < s.t1Ms)
        ? 0
        : Math.abs(s.t0Ms - ms);
    if (g < gap) {
      gap = g;
      best = i;
    }
  });
  return best;
}
