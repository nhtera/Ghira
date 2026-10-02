// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import type { SegmentView } from "../../bindings";
import { buildRows, findMatches, findRanges, fold, lineWords, marksOf, rowIndexBySegment, segmentAt, wordIndexAt, MAX_LINES_PER_GROUP } from "./logic";
import { currentTopic, showTopicRail } from "../topic-rail";

const seg = (i: number, speaker: string | null, over: Partial<SegmentView> = {}): SegmentView => ({
  gid: `s${i}`,
  speakerGid: speaker,
  t0Ms: i * 10_000,
  t1Ms: i * 10_000 + 8_000,
  text: `line ${i}`,
  language: "en",
  confidence: 0.9,
  edited: false,
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
    expect(rows.map((r) => (r.kind === "topic" ? `T:${r.title}` : `G:${r.speakerGid}${r.segs.length}`))).toEqual(["T:Start", "G:a2", "G:b1", "T:Budget", "G:b1", "G:a1"]);
    expect(rowIndexBySegment(rows, segs.length)).toEqual([1, 1, 2, 4, 5]);
  });

  it("cuts a long monologue into paragraphs", () => {
    const segs = Array.from({ length: MAX_LINES_PER_GROUP + 2 }, (_, i) => seg(i, "a"));
    const rows = buildRows(segs, []);
    expect(rows.map((r) => (r.kind === "group" ? r.segs.length : 0))).toEqual([MAX_LINES_PER_GROUP, 2]);
  });

  it("attaches a mark to the line it falls in", () => {
    const m = marksOf([seg(0, "a"), seg(1, "a")], [{ tMs: 12_000, tag: "decision" }]);
    expect([...m.keys()]).toEqual([1]);
  });
});

describe("topic rail", () => {
  it("shows for an hour or for four topics", () => {
    const t = (n: number) => Array.from({ length: n }, (_, i) => ({ title: `t${i}`, tMs: i * 1000 }));
    expect(showTopicRail(30 * 60_000, t(3))).toBe(false);
    expect(showTopicRail(30 * 60_000, t(4))).toBe(true);
    expect(showTopicRail(61 * 60_000, t(1))).toBe(true);
    expect(showTopicRail(61 * 60_000, [])).toBe(false);
  });

  it("tracks the topic being discussed", () => {
    const topics = [{ tMs: 0 }, { tMs: 5000 }, { tMs: 9000 }];
    expect([-1, 0, 4999, 5000, 20_000].map((ms) => currentTopic(topics, ms))).toEqual([-1, 0, 0, 1, 2]);
  });
});
