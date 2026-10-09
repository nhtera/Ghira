// SPDX-License-Identifier: Apache-2.0
// Which block goes in which section of the Notes tab (pure, so it is tested
// without the DOM). Order: Summary, template sections, Decisions, Action
// items, Open questions, Key quotes, Your notes, Topics.
import type {
  ActionItemView,
  MarkedMoment,
  MeetingNotes,
  NoteBlockView,
  TemplateSection,
} from "../../bindings";

export type UserNote = {
  block: NoteBlockView;
  /** The AI's cited expansion of this line (empty text: not found). */ enhanced: NoteBlockView | null;
};

export type NotesLayout = {
  tldr: NoteBlockView[];
  sections: { section: TemplateSection; blocks: NoteBlockView[] }[];
  decisions: NoteBlockView[];
  /** Decisions nobody agreed to yet (block kind `proposal`). */
  proposals: NoteBlockView[];
  actions: ActionItemView[];
  questions: NoteBlockView[];
  quotes: NoteBlockView[];
  mine: UserNote[];
  topics: NoteBlockView[];
  /** Answers saved from Ask (block kind `answer`, pinned). */
  answers: NoteBlockView[];
};

/** Yours: typed by you, or AI text you edited (kept by Regenerate). */
export const isMine = (origin: NoteBlockView["origin"]) => origin !== "ai";

export const enhancedOf = (b: NoteBlockView) =>
  b.kind.startsWith("enhanced:") ? b.kind.slice("enhanced:".length) : null;

/** An id as words (`went_well` → "Went well"): the title of a section whose template is gone. */
export const humanizeSectionId = (id: string) => {
  const words = id.replace(/_/g, " ").trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
};

/**
 * The template's sections, then any `section:<id>` the blocks hold that it
 * does not list (the template was edited or deleted, or the notes came from
 * another device): a section never disappears from the notes.
 */
export function sectionsWithFallback(notes: Pick<MeetingNotes, "sections" | "blocks">): TemplateSection[] {
  const out = [...notes.sections];
  for (const b of notes.blocks) {
    const id = b.kind.startsWith("section:") ? b.kind.slice("section:".length) : null;
    if (id && !out.some((s) => s.id === id)) {
      const title = humanizeSectionId(id);
      out.push({ id, titleEn: title, titleVi: title });
    }
  }
  return out;
}

/** `onlyMine` hides what the app wrote and you did not touch. */
export function layoutNotes(
  notes: MeetingNotes,
  onlyMine: boolean,
): NotesLayout {
  const keep = (o: NoteBlockView["origin"]) => !onlyMine || isMine(o);
  const blocks = notes.blocks;
  const ofKind = (kind: string) =>
    blocks.filter(
      (b) => b.kind === kind && b.origin !== "user" && keep(b.origin),
    );
  const enhanced = new Map<string, NoteBlockView>();
  for (const b of blocks) {
    const of = enhancedOf(b);
    if (of) enhanced.set(of, b);
  }
  return {
    tldr: ofKind("tldr"),
    sections: sectionsWithFallback(notes)
      .map((section) => ({ section, blocks: ofKind(`section:${section.id}`) }))
      .filter((s) => s.blocks.length > 0),
    decisions: ofKind("decision"),
    proposals: ofKind("proposal"),
    actions: notes.actionItems.filter((a) => keep(a.origin)),
    questions: ofKind("question"),
    quotes: ofKind("quote"),
    mine: blocks
      .filter((b) => b.origin === "user")
      .map((block) => {
        const e = enhanced.get(block.gid) ?? null;
        return { block, enhanced: e && keep(e.origin) ? e : null };
      }),
    topics: ofKind("topic"),
    answers: ofKind("answer"),
  };
}

export const hasMine = (notes: MeetingNotes) =>
  notes.blocks.some(
    (b) => b.origin === "user" || (b.origin === "aiEdited" && !enhancedOf(b)),
  ) || notes.actionItems.some((a) => isMine(a.origin));

/** The marks (moments the user marked while recording) a block or action item covers, in time order. */
export const marksCovering = (marks: readonly MarkedMoment[], gid: string) =>
  marks.filter((m) => m.coveredBy.includes(gid)).sort((a, b) => (a.tMs ?? 0) - (b.tMs ?? 0));

/** Marks nothing in the notes covers: the "Moments you marked" section. */
export const uncoveredMarks = (marks: readonly MarkedMoment[]) => marks.filter((m) => m.coveredBy.length === 0);
