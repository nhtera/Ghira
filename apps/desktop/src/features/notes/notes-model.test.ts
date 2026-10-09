// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import type {
  ActionItemView,
  MeetingNotes,
  NoteBlockView,
} from "../../bindings";
import { hasMine, humanizeSectionId, layoutNotes, sectionsWithFallback } from "./notes-model";

const b = (
  gid: string,
  kind: string,
  origin: NoteBlockView["origin"],
  text = gid,
): NoteBlockView => ({ gid, kind, origin, text, pinned: false, citations: [] });
const a = (gid: string, origin: ActionItemView["origin"]): ActionItemView => ({
  gid,
  text: gid,
  ownerSpeakerGid: null,
  dueText: null,
  done: false,
  origin,
  citations: [],
});

const notes: MeetingNotes = {
  marks: [],
  sections: [
    { id: "requests", titleEn: "Client requests", titleVi: "Yêu cầu" },
    { id: "empty", titleEn: "Empty", titleVi: "Trống" },
  ],
  blocks: [
    b("t", "tldr", "ai"),
    b("s", "section:requests", "ai"),
    b("d1", "decision", "ai"),
    b("d2", "decision", "aiEdited"),
    b("q", "question", "ai"),
    b("k", "quote", "ai"),
    b("n1", "note", "user"),
    b("e1", "enhanced:n1", "ai"),
    b("n2", "decision", "user"),
    b("e2", "enhanced:n2", "ai", ""),
    b("top", "topic", "ai"),
  ],
  actionItems: [a("x", "ai"), a("y", "user")],
};

describe("layoutNotes", () => {
  it("puts each block in its section and pairs your lines with their expansion", () => {
    const l = layoutNotes(notes, false);
    expect(l.tldr.map((x) => x.gid)).toEqual(["t"]);
    expect(l.sections.map((s) => s.section.id)).toEqual(["requests"]);
    expect(l.decisions.map((x) => x.gid)).toEqual(["d1", "d2"]);
    expect(l.mine.map((m) => [m.block.gid, m.enhanced?.gid])).toEqual([
      ["n1", "e1"],
      ["n2", "e2"],
    ]);
    expect(l.mine[1]!.enhanced!.text).toBe("");
    expect(l.actions).toHaveLength(2);
    expect(l.topics).toHaveLength(1);
  });

  it("show only mine keeps what you wrote or edited", () => {
    const l = layoutNotes(notes, true);
    expect(l.tldr).toEqual([]);
    expect(l.sections).toEqual([]);
    expect(l.decisions.map((x) => x.gid)).toEqual(["d2"]);
    expect(l.mine.map((m) => [m.block.gid, m.enhanced])).toEqual([
      ["n1", null],
      ["n2", null],
    ]);
    expect(l.actions.map((x) => x.gid)).toEqual(["y"]);
  });

  it("knows when nothing is yours", () => {
    expect(hasMine(notes)).toBe(true);
    expect(
      hasMine({
        ...notes,
        blocks: [b("t", "tldr", "ai")],
        actionItems: [a("x", "ai")],
      }),
    ).toBe(false);
  });
});

describe("sections never disappear", () => {
  const withBlocks = (sections: MeetingNotes["sections"], kinds: string[]): MeetingNotes => ({
    sections,
    blocks: kinds.map((k, i) => b(`b${i}`, k, "ai")),
    actionItems: [],
    marks: [],
  });

  it("a template that no longer lists a section still shows its blocks, titled from the id", () => {
    const l = layoutNotes(withBlocks([{ id: "risks", titleEn: "Risks", titleVi: "Rủi ro" }], ["section:risks", "section:went_well", "section:went_well"]), false);
    expect(l.sections.map((s) => [s.section.id, s.section.titleEn, s.blocks.length])).toEqual([
      ["risks", "Risks", 1],
      ["went_well", "Went well", 2],
    ]);
  });

  it("a deleted template (no sections at all) keeps every section of the notes", () => {
    const l = layoutNotes(withBlocks([], ["tldr", "section:a_b", "section:c"]), false);
    expect(l.sections.map((s) => s.section.id)).toEqual(["a_b", "c"]);
    expect(l.sections[0]!.section.titleVi).toBe("A b");
  });

  it("listed sections come first in the template's order, and the humanizer is plain", () => {
    expect(sectionsWithFallback(withBlocks([{ id: "x", titleEn: "X", titleVi: "X" }], ["section:y", "section:x"])).map((s) => s.id)).toEqual(["x", "y"]);
    expect(humanizeSectionId("quyet_dinh")).toBe("Quyet dinh");
    expect(humanizeSectionId("")).toBe("");
  });
});
