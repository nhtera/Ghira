// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import type { SegmentView } from "../../bindings";
import { buildRows, stackRuns, findMatches, findRanges, fold, lineWords, marksBySegment, rowIndexBySegment, segmentAt, wordIndexAt, MAX_LINES_PER_GROUP } from "./logic";

const seg = (i: number, speaker: string | null, over: Partial<SegmentView> = {}): SegmentView => ({
  gid: `s${i}`,
  speakerGid: speaker,
  t0Ms: i * 10_000,
  t1Ms: i * 10_000 + 8_000,
  text: `line ${i}`,
  language: "en",
  confidence: 0.9,
  edited: false,
  overlap: false,
  words: [],
  ...over,
});

describe("fold and find", () => {
  it("folds accents and đ one unit per unit", () => {
    expect(fold("Nhận diện Đặng")).toBe("nhan dien dang");
    expect(fold("Nhận diện Đặng")).toHaveLength("Nhận diện Đặng".length);
  });

  it("finds without accents and maps back to the original text", () => {
    const text = "Mô hình nhận diện người nói";
    expect(findRanges(text, "nhan dien")).toEqual([[8, 17]]);
    expect(text.slice(8, 17)).toBe("nhận diện");
    expect(findRanges(text, "NHẬN DIỆN")).toEqual([[8, 17]]);
    expect(findRanges(text, "  ")).toEqual([]);
  });

  it("finds every occurrence across lines, in order", () => {
    const m = findMatches([{ text: "Đà Nẵng da" }, { text: "none" }, { text: "da da" }], "da");
    expect(m.map((x) => [x.seg, x.start])).toEqual([[0, 0], [0, 8], [2, 0], [2, 3]]);
  });
});

describe("words and karaoke", () => {
  const s = seg(0, "a", {
    text: "xin chào các bạn",
    words: [0, 1, 2, 3].map((i) => ({ t0Ms: i * 1000, t1Ms: i * 1000 + 900, confidence: i === 1 ? 0.3 : 0.9 })),
  });

  it("aligns timings and confidence with the words", () => {
    const w = lineWords(s);
    expect(w.map((x) => x.offset)).toEqual([0, 4, 9, 13]);
    expect(w.map((x) => x.low)).toEqual([false, true, false, false]);
  });

  it("drops timings when the counts differ (an edited line)", () => {
    expect(lineWords({ ...s, text: "xin chào" }).every((w) => w.t0Ms == null)).toBe(true);
  });

  it("finds the word at a time", () => {
    const w = lineWords(s);
    expect(wordIndexAt(w, -5)).toBe(-1);
    expect(wordIndexAt(w, 0)).toBe(0);
    expect(wordIndexAt(w, 1500)).toBe(1);
    expect(wordIndexAt(w, 99_000)).toBe(3);
    expect(wordIndexAt(lineWords({ ...s, words: [] }), 1500)).toBe(-1);
  });

  it("finds the line at a time, with a short grace after its end", () => {
    const segs = [seg(0, "a"), seg(1, "a"), seg(5, "a")];
    expect(segmentAt(segs, 0)).toBe(0);
    expect(segmentAt(segs, 10_500)).toBe(1);
    expect(segmentAt(segs, 9_000)).toBe(0); // 1 s after the end of line 0
    expect(segmentAt(segs, 30_000)).toBe(-1); // a long gap
    expect(segmentAt(segs, 51_000)).toBe(2);
  });
});

describe("rows", () => {
  it("groups consecutive lines of one speaker and puts topics between", () => {
    const segs = [seg(0, "a"), seg(1, "a"), seg(2, "b"), seg(3, "b"), seg(4, "a")];
    const rows = buildRows(segs, [{ title: "Budget", tMs: 25_000 }, { title: "Start", tMs: 0 }]);
    expect(rows.map((r) => (r.kind === "topic" ? `T:${r.title}` : r.kind === "stack" ? "S" : `G:${r.speakerGid}${r.segs.length}`))).toEqual(["T:Start", "G:a2", "G:b1", "T:Budget", "G:b1", "G:a1"]);
    expect(rowIndexBySegment(rows, segs.length)).toEqual([1, 1, 2, 4, 5]);
  });

  it("cuts a long monologue into paragraphs", () => {
    const segs = Array.from({ length: MAX_LINES_PER_GROUP + 2 }, (_, i) => seg(i, "a"));
    const rows = buildRows(segs, []);
    expect(rows.map((r) => (r.kind === "group" ? r.segs.length : 0))).toEqual([MAX_LINES_PER_GROUP, 2]);
  });

  it("puts a mark on the line the core assigned it to, and none on a mark in silence", () => {
    const segs = [seg(0, "a"), seg(1, "a")];
    const m = marksBySegment(segs, [
      { tMs: 12_000, tag: "decision", segment: segs[1]!.gid },
      { tMs: 99_000, tag: "star", segment: null },
      { tMs: 5_000, tag: "star", segment: "gone" },
    ]);
    expect([...m.keys()]).toEqual([1]);
    expect(m.get(1)).toHaveLength(1);
  });
});

describe("stacking talked-over lines", () => {
  const sp = (speaker: string, t0Ms: number, t1Ms: number, overlap = false) => ({ speaker, t0Ms, t1Ms, overlap });
  it("stacks a flagged line with the line it overlaps", () => {
    expect(stackRuns([sp("a", 0, 5000, true), sp("b", 3000, 8000, true), sp("a", 9000, 12_000)])).toEqual([[0, 2]]);
  });
  it("normal turn-taking, even with a hair of overlap, is not stacked", () => {
    const talk = [sp("a", 0, 5000), sp("b", 4900, 9000), sp("a", 8950, 12_000), sp("b", 11_900, 15_000)];
    expect(stackRuns(talk)).toEqual([]);
  });
  it("a monologue with a flagged mm-hm stacks only the turn it interrupts", () => {
    const spans = [sp("a", 0, 20_000), sp("a", 20_000, 40_000), sp("b", 25_000, 26_000, true), sp("a", 40_000, 60_000), sp("a", 60_000, 80_000)];
    expect(stackRuns(spans)).toEqual([[1, 3]]);
  });
  it("chains flagged lines that share a neighbour, and cuts a run at the cap", () => {
    expect(stackRuns([sp("a", 0, 5000, true), sp("b", 4000, 9000, true), sp("c", 8000, 12_000, true), sp("a", 20_000, 21_000)])).toEqual([[0, 3]]);
    // One long turn that nine others flag themselves over: at most MAX_LINES_PER_GROUP lines per run.
    const big = [sp("a", 0, 100_000), ...Array.from({ length: 9 }, (_, i) => sp(i % 2 ? "b" : "c", 1000 + i * 10_000, 5000 + i * 10_000, true))];
    const runs = stackRuns(big);
    expect(runs.length).toBeGreaterThan(0);
    expect(runs.every(([a, b]) => b - a <= MAX_LINES_PER_GROUP)).toBe(true);
  });
  it("never stacks one speaker with themself, and ignores lines that only touch", () => {
    expect(stackRuns([sp("a", 0, 5000, true), sp("a", 3000, 8000, true)])).toEqual([]);
    expect(stackRuns([sp("a", 0, 5000, true), sp("b", 5000, 8000, true)])).toEqual([]);
  });
  it("respects cuts and tolerates missing times", () => {
    const spans = [sp("a", 0, 5000, true), sp("b", 1000, 3000, true), sp("a", 10_000, 15_000, true), sp("b", 11_000, 12_000, true)];
    expect(stackRuns(spans)).toEqual([
      [0, 2],
      [2, 4],
    ]);
    expect(stackRuns(spans, [1])).toEqual([[2, 4]]);
    expect(stackRuns([sp("a", 0, 0, true), { speaker: "b", t0Ms: null, t1Ms: null, overlap: true }])).toEqual([]);
  });

  const segs = [seg(0, "a"), seg(1, "a", { t0Ms: 12_000, overlap: true }), seg(2, "b", { t0Ms: 14_000, t1Ms: 20_000, overlap: true }), seg(3, "a", { t0Ms: 30_000, t1Ms: 33_000 })];
  it("flagged lines become one stack row of paragraphs, in any meeting mode", () => {
    const rows = buildRows(segs, []);
    expect(rows.map((r) => r.kind)).toEqual(["group", "stack", "group"]);
    const stack = rows[1]!;
    if (stack.kind !== "stack") throw new Error("stack");
    expect(stack.first).toBe(1);
    expect(stack.groups.map((g) => `${g.speakerGid}${g.segs.length}@${g.first}`)).toEqual(["a1@1", "b1@2"]);
    // Lines keep their identity and place for scrolling.
    expect(rowIndexBySegment(rows, segs.length)).toEqual([0, 1, 1, 2]);
  });
  it("overlapping in time without the core's flag is plain speech", () => {
    const plain = segs.map((s) => ({ ...s, overlap: false }));
    expect(buildRows(plain, []).every((r) => r.kind !== "stack")).toBe(true);
  });
  it("a topic header between two lines cuts the stack", () => {
    const rows = buildRows(segs, [{ title: "New", tMs: 14_000 }]);
    expect(rows.map((r) => r.kind)).toEqual(["group", "topic", "group", "group"]);
  });
});
