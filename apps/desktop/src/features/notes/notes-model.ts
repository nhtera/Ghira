// SPDX-License-Identifier: Apache-2.0
// Which block goes in which section of the Notes tab (pure, so it is tested
// without the DOM). Order: Summary, template sections, Decisions, Action
// items, Open questions, Key quotes, Your notes, Topics.
import type {
  ActionItemView,
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
  actions: ActionItemView[];
  questions: NoteBlockView[];
  quotes: NoteBlockView[];
  mine: UserNote[];
  topics: NoteBlockView[];
};

/** Yours: typed by you, or AI text you edited (kept by Regenerate). */
export const isMine = (origin: NoteBlockView["origin"]) => origin !== "ai";

export const enhancedOf = (b: NoteBlockView) =>
  b.kind.startsWith("enhanced:") ? b.kind.slice("enhanced:".length) : null;

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
    sections: notes.sections
      .map((section) => ({ section, blocks: ofKind(`section:${section.id}`) }))
      .filter((s) => s.blocks.length > 0),
    decisions: ofKind("decision"),
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
  };
}

export const hasMine = (notes: MeetingNotes) =>
  notes.blocks.some(
    (b) => b.origin === "user" || (b.origin === "aiEdited" && !enhancedOf(b)),
  ) || notes.actionItems.some((a) => isMine(a.origin));
