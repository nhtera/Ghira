// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import type { MeetingSpeaker, SegmentView } from "../../bindings";
import { isPanelError, linesFrom, mergeTargets, ownLines, splitAllowed, splitArgs, splitCount } from "./logic";

const sp = (gid: string, over: Partial<MeetingSpeaker> = {}): MeetingSpeaker => ({
  gid, name: null, number: 1, colorSlot: 1, isMe: false, notPerson: false, lines: 1, sampleT0Ms: null, sampleT1Ms: null, suggestion: null, ...over,
});
const seg = (gid: string, speakerGid: string, t0Ms: number): SegmentView => ({
  gid, speakerGid, t0Ms, t1Ms: t0Ms + 1000, text: gid, language: null, confidence: null, edited: false, overlap: false, words: [],
});

describe("mergeTargets", () => {
  it("leaves out the speaker and anyone who is not a person", () => {
    const all = [sp("a"), sp("b"), sp("c", { notPerson: true }), sp("d", { isMe: true })];
    expect(mergeTargets(all, "a").map((s) => s.gid)).toEqual(["b", "d"]);
  });
});

describe("lines", () => {
  const segs = [seg("l3", "a", 30), seg("l1", "a", 10), seg("x", "b", 15), seg("l2", "a", 20)];
  const own = ownLines(segs, "a");
  it("are the speaker's own, in time order", () => expect(own.map((s) => s.gid)).toEqual(["l1", "l2", "l3"]));
  it("from a line: it and the later ones", () => {
    expect(linesFrom(own, "l2").map((s) => s.gid)).toEqual(["l2", "l3"]);
    expect(linesFrom(own, "x")).toEqual([]);
    expect(linesFrom(own, null)).toEqual([]);
  });
  it("split arguments set exactly one of the two", () => {
    expect(splitArgs({ mode: "from", from: "l2" })).toEqual({ segmentGids: [], fromSegment: "l2" });
    expect(splitArgs({ mode: "lines", picked: ["l1"] })).toEqual({ segmentGids: ["l1"], fromSegment: null });
  });
  it("a split moves some lines, never none and never all", () => {
    expect(splitCount(own, { mode: "from", from: "l2" })).toBe(2);
    expect(splitAllowed(own, { mode: "from", from: "l2" })).toBe(true);
    expect(splitAllowed(own, { mode: "from", from: "l1" })).toBe(false);
    expect(splitAllowed(own, { mode: "lines", picked: [] })).toBe(false);
    expect(splitAllowed(own, { mode: "lines", picked: ["l1", "l3"] })).toBe(true);
    expect(splitAllowed(own, { mode: "lines", picked: ["l1", "l2", "l3"] })).toBe(false);
  });
});

describe("isPanelError", () => {
  it("knows the core's codes and nothing else", () => {
    for (const c of ["liveMeeting", "notASpeaker", "sameSpeaker", "farSide", "nothingToSplit", "wholeSpeaker", "isMe", "storage"]) expect(isPanelError(c)).toBe(true);
    expect(isPanelError("busyRecording")).toBe(false);
  });
});
