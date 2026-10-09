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
  MarkedMoment,
  MarkView,
  MeetingSpeaker,
  NoteBlockView,
  SegmentView,
  TemplateSection,
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
  | "tldr"
  | "decision"
  | "question"
  | "topic"
  | "quote"
  | "answer"
  | "note"
  | "other";
export const SECTION_ORDER: SectionKey[] = [
  "tldr",
  "decision",
  "question",
  "topic",
  "quote",
  "answer",
  "note",
  "other",
];

export function sectionOf(kind: string): SectionKey {
  if (kind.startsWith("enhanced:") || kind === "action") return "note";
  // A proposed decision is listed under Decisions, after the decided ones.
  if (kind === "proposal") return "decision";
  // A template's own section is grouped by its id (see `groupBlocks`), not as "More".
  return (SECTION_ORDER as string[]).includes(kind)
    ? (kind as SectionKey)
    : "other";
}

export type BlockGroup = {
  /** A fixed section, or `section:<id>` for one of the meeting template's own. */
  key: string;
  /** The heading of a template's section (the fixed ones are named by locale key). */
  title?: string;
  blocks: NoteBlockView[];
};

/** An id as words (`went_well` → "Went well"): the title of a section whose template this device does not have. */
export const humanizeSectionId = (id: string) => {
  const words = id.replace(/_/g, " ").trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
};

/**
 * Blocks grouped by section in reading order; empty sections are left out.
 * The template's own sections (`section:<id>`) come after the summary, titled
 * from `sections` when the notes list it, else from the id: a section whose
 * template was edited or deleted, or that was made on the computer with a
 * template this phone does not have, still shows.
 */
export function groupBlocks(
  blocks: NoteBlockView[],
  sections: readonly TemplateSection[] = [],
  vi = false,
): BlockGroup[] {
  const fixed: BlockGroup[] = SECTION_ORDER.map((key) => ({
    key,
    blocks: blocks
      .filter((b) => !b.kind.startsWith("section:") && sectionOf(b.kind) === key)
      // Decided first, then the proposed ones (the sort is stable).
      .sort((a, b) => Number(a.kind === "proposal") - Number(b.kind === "proposal")),
  })).filter((g) => g.blocks.length > 0);
  const ids: string[] = [];
  for (const b of blocks)
    if (b.kind.startsWith("section:")) {
      const id = b.kind.slice("section:".length);
      if (!ids.includes(id)) ids.push(id);
    }
  // The template's order first, then any the template does not list.
  const order = [
    ...sections.map((s) => s.id).filter((id) => ids.includes(id)),
    ...ids.filter((id) => !sections.some((s) => s.id === id)),
  ];
  const own: BlockGroup[] = order.map((id) => {
    const s = sections.find((x) => x.id === id);
    return {
      key: `section:${id}`,
      title: s ? (vi ? s.titleVi : s.titleEn) : humanizeSectionId(id),
      blocks: blocks.filter((b) => b.kind === `section:${id}`),
    };
  });
  const at = fixed.findIndex((g) => g.key === "tldr") + 1;
  return [...fixed.slice(0, at), ...own, ...fixed.slice(at)];
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

/**
 * `numbered` is the localized "Speaker N", `me` the localized "Me" (the phone's
 * owner, until they are given a name). Color slot plus initial: never color alone.
 */
export function transcriptSpeaker(
  s: MeetingSpeaker | undefined,
  numbered: (n: number) => string,
  me?: string,
): TranscriptSpeaker | null {
  if (!s) return null;
  if (!s.name && s.isMe && me)
    return { label: me, colorSlot: s.colorSlot, isMe: true };
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

/** Marks nothing in the notes covers: "Moments you marked". */
export const uncoveredMarks = (marks: readonly MarkedMoment[]) =>
  marks.filter((m) => m.coveredBy.length === 0);

/** The marks of each transcript line, by segment gid (the core picks the line; a mark in silence has none). */
export function marksBySegment(
  marks: readonly MarkView[],
): Map<string, MarkView[]> {
  const by = new Map<string, MarkView[]>();
  for (const m of marks)
    if (m.segment !== null) by.set(m.segment, [...(by.get(m.segment) ?? []), m]);
  return by;
}
