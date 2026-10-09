// SPDX-License-Identifier: Apache-2.0
// The mind map's data: the meeting's notes as a tree (title -> sections ->
// items). Pure and typed structurally (no app bindings) so it can move to
// @ghi/ui; the caller passes the already-localized section titles. Nothing is
// invented: every leaf is a note sentence, an action, a topic or a marked
// moment, with its first citation as the thing to play.

/** The part of a citation the map needs. */
export type TreeCite = { t0Ms: number | null; t1Ms: number | null; missing?: boolean };
export type TreeBlock = { gid: string; text: string; citations: readonly TreeCite[] };
export type TreeAction = TreeBlock & { ownerSpeakerGid: string | null; dueText?: string | null; done?: boolean };

export type SectionKind = "summary" | "template" | "decisions" | "proposed" | "actions" | "questions" | "topics" | "answers" | "marked" | "other";

export type NotesTreeInput = {
  /** The meeting's title: the root. */
  title: string;
  /** Section titles, localized by the caller. */
  titles: { summary: string; decisions: string; proposed: string; actions: string; questions: string; topics: string; answers: string; marked: string; other: string };
  tldr: readonly TreeBlock[];
  /** Template sections, in the template's order. */
  sections: readonly { id: string; title: string; blocks: readonly TreeBlock[] }[];
  decisions: readonly TreeBlock[];
  actions: readonly TreeAction[];
  questions: readonly TreeBlock[];
  topics: readonly TreeBlock[];
  /** Answers saved from Ask (block kind `answer`). */
  answers?: readonly TreeBlock[];
  /** Suggested follow-ups (block kind `proposal`). */
  proposals?: readonly TreeBlock[];
  /** Moments the person marked while recording. */
  marks?: readonly { gid: string; text: string; tMs: number }[];
  /** gids of items that cover a marked moment (drawn with a star). */
  covered?: ReadonlySet<string>;
  /** Blocks of a kind this version does not know: kept, in their own section. */
  other?: readonly TreeBlock[];
};

export type MapNode = {
  /** Stable across renders (collapse state keys on it). */
  id: string;
  kind: "root" | "section" | "leaf";
  sectionKind?: SectionKind;
  /** What is drawn: at most `LEAF_MAX` characters. */
  text: string;
  /** The whole text (tooltip). */
  full: string;
  children: MapNode[];
  /** Where a click goes: the first citation. */
  cite?: TreeCite;
  /** A marked moment's time, when there is no citation. */
  atMs?: number;
  proposed?: boolean;
  starred?: boolean;
  ownerGid?: string | null;
  done?: boolean;
  due?: string | null;
};

export const LEAF_MAX = 80;

/** At most `max` characters (code points), cut at a word where it can and ended with an ellipsis. */
export function clip(text: string, max = LEAF_MAX): string {
  const t = text.replace(/\s+/g, " ").trim();
  const chars = Array.from(t);
  if (chars.length <= max) return t;
  const cut = chars.slice(0, max - 1).join("");
  const sp = cut.lastIndexOf(" ");
  return `${(sp > max * 0.6 ? cut.slice(0, sp) : cut).trimEnd()}…`;
}

const firstCite = (b: Pick<TreeBlock, "citations">) => b.citations.find((c) => c.t0Ms != null && !c.missing) ?? b.citations[0];

const leaf = (b: TreeBlock, covered: ReadonlySet<string> | undefined, extra: Partial<MapNode> = {}): MapNode => ({
  id: b.gid,
  kind: "leaf",
  text: clip(b.text),
  full: b.text,
  children: [],
  cite: firstCite(b),
  starred: covered?.has(b.gid) || undefined,
  ...extra,
});

/** The tree of the notes. Empty sections are left out; the root always exists. */
export function notesToTree(input: NotesTreeInput): MapNode {
  const { titles, covered } = input;
  const children: MapNode[] = [];
  const section = (id: string, sectionKind: SectionKind, title: string, items: MapNode[]) => {
    if (items.length) children.push({ id: `sec:${id}`, kind: "section", sectionKind, text: clip(title), full: title, children: items });
  };
  const blocks = (list: readonly TreeBlock[], extra?: Partial<MapNode>) => list.filter((b) => b.text.trim()).map((b) => leaf(b, covered, extra));

  section("summary", "summary", titles.summary, blocks(input.tldr));
  for (const s of input.sections) section(`template:${s.id}`, "template", s.title, blocks(s.blocks));
  section("decisions", "decisions", titles.decisions, blocks(input.decisions));
  section("proposed", "proposed", titles.proposed, blocks(input.proposals ?? [], { proposed: true }));
  section(
    "actions",
    "actions",
    titles.actions,
    input.actions
      .filter((a) => a.text.trim())
      .map((a) => leaf(a, covered, { ownerGid: a.ownerSpeakerGid, done: a.done || undefined, due: a.dueText ?? null })),
  );
  section("questions", "questions", titles.questions, blocks(input.questions));
  section("topics", "topics", titles.topics, blocks(input.topics));
  section("answers", "answers", titles.answers, blocks(input.answers ?? []));
  section(
    "marked",
    "marked",
    titles.marked,
    (input.marks ?? []).map((m) => ({ id: m.gid, kind: "leaf" as const, text: clip(m.text), full: m.text, children: [], atMs: m.tMs, cite: { t0Ms: m.tMs, t1Ms: m.tMs } })),
  );
  section("other", "other", titles.other, blocks(input.other ?? []));
  return { id: "root", kind: "root", text: clip(input.title || titles.summary, 60), full: input.title, children };
}

/** Characters that would turn text into Markdown syntax, escaped (the same set the Markdown export guards). */
export function escapeMd(text: string): string {
  const t = text.replace(/\s+/g, " ").trim().replace(/([\\`*_[\]<>&|])/g, "\\$1");
  return t.replace(/^([#>+\-=~]|\d+[.)])/, "\\$1");
}

/**
 * The tree as a nested Markdown list (the title is the first line); actions
 * read like the Markdown export: `- [ ] text — Owner (due X)`. `ownerName`
 * turns a speaker gid into a name.
 */
export function treeToOutline(root: MapNode, ownerName: (gid: string) => string | null = () => null): string {
  const lines = [`# ${escapeMd(root.full || root.text)}`, ""];
  const walk = (nodes: readonly MapNode[], depth: number) => {
    for (const n of nodes) {
      const star = n.starred ? " ★" : "";
      const isAction = n.kind === "leaf" && n.ownerGid !== undefined;
      const tick = isAction ? (n.done ? "[x] " : "[ ] ") : "";
      const owner = isAction && n.ownerGid ? ownerName(n.ownerGid) : null;
      const who = owner ? ` — ${escapeMd(owner)}` : "";
      const due = isAction && n.due ? ` (due ${escapeMd(n.due)})` : "";
      const text = n.kind === "section" ? `**${escapeMd(n.full)}**` : escapeMd(n.full);
      lines.push(`${"  ".repeat(depth)}- ${tick}${text}${who}${due}${star}`);
      walk(n.children, depth + 1);
    }
  };
  walk(root.children, 0);
  return `${lines.join("\n")}\n`;
}
