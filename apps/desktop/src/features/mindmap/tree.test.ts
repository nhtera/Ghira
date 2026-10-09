// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { clip, escapeMd, notesToTree, treeToOutline, type NotesTreeInput, type TreeBlock } from "./tree";

const cite = (t0Ms: number | null, missing = false) => ({ t0Ms, t1Ms: t0Ms == null ? null : t0Ms + 2000, missing });
const block = (gid: string, text: string, at: number | null = 1000): TreeBlock => ({ gid, text, citations: at == null ? [] : [cite(at)] });

const titles = { summary: "Summary", decisions: "Decisions", proposed: "Proposed", actions: "Action items", questions: "Open questions", topics: "Topics", marked: "Moments you marked", other: "Other" };
const input = (over: Partial<NotesTreeInput> = {}): NotesTreeInput => ({
  title: "Client call",
  titles,
  tldr: [block("t1", "We ship on the 12th.")],
  sections: [],
  decisions: [block("d1", "Rename ships in beta.", 5000)],
  actions: [{ ...block("a1", "Send the deck", 7000), ownerSpeakerGid: "s1", dueText: "Fri", done: false }],
  questions: [],
  topics: [block("p1", "Kickoff", 0)],
  ...over,
});

describe("notesToTree", () => {
  it("is the title, then non-empty sections in notes order, each leaf with its first citation", () => {
    const root = notesToTree(input());
    expect(root.kind).toBe("root");
    expect(root.text).toBe("Client call");
    expect(root.children.map((s) => s.full)).toEqual(["Summary", "Decisions", "Action items", "Topics"]);
    const dec = root.children[1]!.children[0]!;
    expect(dec).toMatchObject({ id: "d1", kind: "leaf", cite: { t0Ms: 5000 } });
  });

  it("puts template sections after Summary and keeps their order", () => {
    const root = notesToTree(input({ sections: [{ id: "x", title: "Risks", blocks: [block("r1", "Late vendor")] }, { id: "y", title: "Empty", blocks: [] }] }));
    expect(root.children.map((s) => s.full).slice(0, 3)).toEqual(["Summary", "Risks", "Decisions"]);
  });

  it("action leaves carry owner, due and done", () => {
    const a = notesToTree(input()).children.find((s) => s.sectionKind === "actions")!.children[0]!;
    expect(a).toMatchObject({ ownerGid: "s1", due: "Fri" });
    expect(a.done).toBeUndefined();
  });

  it("truncates a leaf to 80 characters and keeps the full text", () => {
    const long = "Một câu rất dài về kế hoạch phát hành bản beta và các việc cần làm trước khi ra mắt cho khách hàng đầu tiên";
    const leaf = notesToTree(input({ tldr: [block("t1", long)] })).children[0]!.children[0]!;
    expect(Array.from(leaf.text).length).toBeLessThanOrEqual(80);
    expect(leaf.text.endsWith("…")).toBe(true);
    expect(leaf.full).toBe(long);
  });

  it("a Proposed section (block kind proposal) marks its leaves", () => {
    const root = notesToTree(input({ proposals: [block("pr1", "Schedule a follow-up")] }));
    const sec = root.children.find((s) => s.sectionKind === "proposed")!;
    expect(sec.full).toBe("Proposed");
    expect(sec.children[0]!.proposed).toBe(true);
    // after Decisions, before Action items
    const names = root.children.map((s) => s.sectionKind);
    expect(names.indexOf("proposed")).toBe(names.indexOf("decisions") + 1);
    expect(names.indexOf("proposed")).toBeLessThan(names.indexOf("actions"));
  });

  it("stars covered items and lists marked moments in their own section", () => {
    const root = notesToTree(input({ covered: new Set(["d1", "a1"]), marks: [{ gid: "m1", text: "Decision at 12:03", tMs: 723_000 }] }));
    const flat = root.children.flatMap((s) => s.children);
    expect(flat.filter((n) => n.starred).map((n) => n.id).sort()).toEqual(["a1", "d1"]);
    const marked = root.children.find((s) => s.sectionKind === "marked")!;
    expect(marked.full).toBe("Moments you marked");
    expect(marked.children[0]).toMatchObject({ id: "m1", atMs: 723_000, cite: { t0Ms: 723_000 } });
  });

  it("unknown kinds do not crash: they land in Other", () => {
    const root = notesToTree(input({ other: [block("o1", "A block of a future kind")] }));
    const other = root.children.at(-1)!;
    expect(other).toMatchObject({ sectionKind: "other", full: "Other" });
    expect(other.children[0]!.id).toBe("o1");
  });

  it("an item without any citation is a leaf with nothing to play; blank texts are dropped", () => {
    const root = notesToTree(input({ tldr: [block("t1", "No source", null), block("t2", "   ")] }));
    const leaves = root.children[0]!.children;
    expect(leaves).toHaveLength(1);
    expect(leaves[0]!.cite).toBeUndefined();
  });

  it("prefers the first citation that can be played over a missing one", () => {
    const b: TreeBlock = { gid: "x", text: "t", citations: [cite(null, true), cite(9000)] };
    expect(notesToTree(input({ tldr: [b] })).children[0]!.children[0]!.cite?.t0Ms).toBe(9000);
  });

  it("empty notes are a lone root", () => {
    const root = notesToTree(input({ tldr: [], decisions: [], actions: [], topics: [] }));
    expect(root.children).toEqual([]);
  });
});

describe("treeToOutline and clip", () => {
  it("is a nested Markdown list matching the notes", () => {
    const md = treeToOutline(notesToTree(input({ covered: new Set(["d1"]) })));
    expect(md).toBe(
      [
        "# Client call",
        "",
        "- **Summary**",
        "  - We ship on the 12th.",
        "- **Decisions**",
        "  - Rename ships in beta. ★",
        "- **Action items**",
        "  - [ ] Send the deck (due Fri)",
        "- **Topics**",
        "  - Kickoff",
        "",
      ].join("\n"),
    );
  });

  it("clip keeps short text and cuts at a word", () => {
    expect(clip("short")).toBe("short");
    const c = clip("alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma", 30);
    expect(Array.from(c).length).toBeLessThanOrEqual(30);
    expect(c).toMatch(/…$/);
    expect(c).not.toMatch(/\s…$/);
  });

});

describe("outline owners and escaping", () => {
  it("action lines carry the owner like the Markdown export", () => {
    const md = treeToOutline(notesToTree(input()), (g) => (g === "s1" ? "Sarah" : null));
    expect(md).toContain("  - [ ] Send the deck — Sarah (due Fri)");
  });

  it("escapes Markdown syntax in notes text", () => {
    expect(escapeMd("a *b* [c] <d> _e_ `f` & g")).toBe("a \\*b\\* \\[c\\] \\<d\\> \\_e\\_ \\`f\\` \\& g");
    expect(escapeMd("# not a heading")).toBe("\\# not a heading");
    expect(escapeMd("1. not a list")).toBe("\\1. not a list");
    const md = treeToOutline(notesToTree(input({ tldr: [block("t1", "- *bold* move")] })));
    expect(md).toContain("  - \\- \\*bold\\* move");
  });
});
