// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import type { Citation, MarkedMoment, MarkView, MeetingSpeaker, SegmentView } from "../../bindings";
import {
  marksBySegment,
  uncoveredMarks,
  activeSegment,
  activeWord,
  groupBlocks,
  noteKind,
  segmentNear,
  segmentWords,
  toNoteCitation,
  transcriptSpeaker,
} from "./notes-model";

const block = (
  kind: string,
  origin: "user" | "ai" | "aiEdited" = "ai",
  text = "x",
) => ({ gid: kind, kind, origin, text, pinned: false, citations: [] });
const seg = (
  gid: string,
  t0Ms: number,
  t1Ms: number,
  text = "a b c",
  conf: number[] = [],
): SegmentView => ({
  gid,
  speakerGid: null,
  t0Ms,
  t1Ms,
  text,
  language: null,
  confidence: null,
  edited: false,
  overlap: false,
  words: conf.map((c, i) => ({
    t0Ms: t0Ms + i * 1000,
    t1Ms: t0Ms + (i + 1) * 1000,
    confidence: c,
  })),
});

describe("noteKind", () => {
  it("maps the origin to provenance", () => {
    expect(noteKind(block("tldr", "ai"))).toBe("ai");
    expect(noteKind(block("note", "user"))).toBe("user");
    expect(noteKind(block("decision", "aiEdited"))).toBe("edited");
  });

  it("calls an empty AI expansion not found", () => {
    expect(noteKind(block("enhanced:n4", "ai", ""))).toBe("missing");
    expect(noteKind(block("enhanced:n4", "ai", "More"))).toBe("ai");
  });
});

describe("groupBlocks", () => {
  it("orders sections for reading and drops empty ones", () => {
    const g = groupBlocks([
      block("note", "user"),
      block("decision"),
      block("tldr"),
      block("section:risks"),
      block("enhanced:n1"),
      block("answer"),
    ]);
    expect(g.map((x) => [x.key, x.blocks.length])).toEqual([
      ["tldr", 1],
      ["decision", 1],
      ["answer", 1],
      ["note", 2],
      ["other", 1],
    ]);
  });
});

describe("toNoteCitation", () => {
  const c = (t0Ms: number | null, missing = false): Citation => ({
    t0Ms,
    t1Ms: null,
    quote: "",
    speakerGid: null,
    stale: false,
    missing,
  });
  it("shows the time, or the number without one", () => {
    expect(toNoteCitation(c(5000), 0, new Set(), "k")).toMatchObject({
      timeMs: 5000,
      broken: false,
      visited: false,
    });
    expect(toNoteCitation(c(null, true), 2, new Set(["k"]), "k")).toMatchObject(
      { index: 3, broken: true, visited: true },
    );
  });
});

describe("transcriptSpeaker", () => {
  const sp = (name: string | null): MeetingSpeaker => ({
    gid: "g",
    name,
    number: 2,
    colorSlot: 4,
    isMe: false,
    notPerson: false,
    lines: 1,
    sampleT0Ms: null,
    sampleT1Ms: null,
    suggestion: null,
  });
  it("labels an unnamed speaker by number and keeps color plus initial", () => {
    expect(transcriptSpeaker(sp(null), (n) => `Speaker ${n}`)).toMatchObject({
      label: "Speaker 2",
      initial: "2",
      colorSlot: 4,
    });
    expect(transcriptSpeaker(sp("Linh"), () => "")).toEqual({
      label: "Linh",
      colorSlot: 4,
      isMe: false,
    });
    expect(transcriptSpeaker(undefined, () => "")).toBeNull();
  });
  it("calls the phone's owner Me until they are named", () => {
    const me = { ...sp(null), isMe: true };
    expect(transcriptSpeaker(me, (n) => `Speaker ${n}`, "Me")).toEqual({
      label: "Me",
      colorSlot: 4,
      isMe: true,
    });
    expect(transcriptSpeaker({ ...me, name: "An" }, () => "", "Me")).toMatchObject({ label: "An" });
  });
});

describe("transcript timing", () => {
  const s = seg("s", 10_000, 13_000, "a b c", [0.9, 0.3, 0.9]);

  it("flags low-confidence words only when words line up", () => {
    expect(segmentWords(s).map((w) => w.lowConfidence)).toEqual([
      false,
      true,
      false,
    ]);
    expect(
      segmentWords({ ...s, words: [] }).map((w) => w.lowConfidence),
    ).toEqual([false, false, false]);
  });

  it("finds the line and the word being played", () => {
    const all = [seg("a", 0, 5000), s];
    expect(activeSegment(all, 11_500)).toBe(1);
    expect(activeSegment(all, 99_000)).toBe(-1);
    expect(activeWord(s, 11_500)).toBe(1);
    expect(activeWord(s, 50)).toBeUndefined();
  });

  it("finds the line nearest a search hit", () => {
    const all = [seg("a", 0, 5000), seg("b", 20_000, 25_000)];
    expect(segmentNear(all, 21_000)).toBe(1);
    expect(segmentNear(all, 8000)).toBe(0);
    expect(segmentNear([], 1)).toBe(-1);
  });
});

describe("marks", () => {
  const moment = (coveredBy: string[]): MarkedMoment => ({ tMs: 1000, tag: "star", segment: "s1", text: "x", coveredBy });

  it("lists only the marks nothing covers", () => {
    expect(uncoveredMarks([moment(["n1"]), moment([]), moment(["a1", "n2"])])).toHaveLength(1);
    expect(uncoveredMarks([])).toEqual([]);
  });

  it("groups marks by the line the core put them on and drops marks in silence", () => {
    const marks: MarkView[] = [
      { tMs: 1000, tag: "decision", segment: "s2" },
      { tMs: 2000, tag: "star", segment: "s2" },
      { tMs: 9000, tag: "star", segment: null },
    ];
    const by = marksBySegment(marks);
    expect([...by.keys()]).toEqual(["s2"]);
    expect(by.get("s2")).toHaveLength(2);
  });
});
