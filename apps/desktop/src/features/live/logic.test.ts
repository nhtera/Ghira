// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import type { LineInfo, SpeakerInfo } from "../../bindings";
import { FOLLOW_SLOP_PX, formatBytes, isFollowing, laneModel, liveBlocks, minutesLeft, parseNoteLine, splitOthers, waitingForAudio } from "./logic";

const sp = (id: number, over: Partial<SpeakerInfo> = {}): SpeakerInfo => ({ id, label: `Speaker ${id}`, colorSlot: ((id - 1) % 8) + 1, isMe: false, provisional: false, notPerson: false, others: false, ...over });
const line = (speaker: number | null, t0Ms: number, t1Ms: number): LineInfo => ({ gid: `${t0Ms}`, speaker, t0Ms, t1Ms, text: "x", overlap: false, words: [] });

describe("auto-scroll follow", () => {
  const el = (scrollTop: number) => ({ scrollTop, clientHeight: 400, scrollHeight: 1000 });
  it("follows at the bottom and within the slop", () => {
    expect(isFollowing(el(600))).toBe(true);
    expect(isFollowing(el(600 - FOLLOW_SLOP_PX))).toBe(true);
  });
  it("stops once the user scrolls up", () => {
    expect(isFollowing(el(600 - FOLLOW_SLOP_PX - 1))).toBe(false);
    expect(isFollowing(el(0))).toBe(false);
  });
});

describe("waitingForAudio", () => {
  const base = { recording: true, asleep: false, lastLevelAtMs: 1000 };
  it("needs 10 s without levels", () => {
    expect(waitingForAudio({ ...base, nowMs: 10_999 })).toBe(false);
    expect(waitingForAudio({ ...base, nowMs: 11_000 })).toBe(true);
  });
  it("is quiet while paused, stopped or asleep", () => {
    expect(waitingForAudio({ ...base, recording: false, nowMs: 99_000 })).toBe(false);
    expect(waitingForAudio({ ...base, asleep: true, nowMs: 99_000 })).toBe(false);
  });
});

describe("laneModel", () => {
  const label = (s: SpeakerInfo) => s.label;
  it("merges close turns of one speaker", () => {
    const { segments } = laneModel([sp(1)], [line(1, 0, 1000), line(1, 1500, 3000), line(1, 9000, 10_000)], label, "Others");
    expect(segments).toEqual([
      { speaker: 1, t0Ms: 0, t1Ms: 3000 },
      { speaker: 1, t0Ms: 9000, t1Ms: 10_000 },
    ]);
  });
  it("folds speakers past eight into one Others lane", () => {
    const speakers = Array.from({ length: 10 }, (_, i) => sp(i + 1));
    const { lanes, segments } = laneModel(speakers, [line(9, 0, 1000), line(10, 5000, 6000), line(1, 7000, 8000)], label, "Others");
    expect(lanes).toHaveLength(9);
    expect(lanes.at(-1)).toEqual({ id: 0, label: "Others", colorSlot: 0 });
    expect(segments.filter((s) => s.speaker === 0)).toHaveLength(2);
  });
  it("has no Others lane up to eight", () => {
    expect(laneModel(Array.from({ length: 8 }, (_, i) => sp(i + 1)), [], label, "Others").lanes).toHaveLength(8);
  });
  it("skips lines without a speaker or time", () => {
    expect(laneModel([sp(1)], [line(null, 0, 1), { ...line(1, 0, 1), t0Ms: null }], label, "Others").segments).toEqual([]);
  });
});

describe("splitOthers", () => {
  it("gives the first eight their own place and the rest to Others", () => {
    const { own, others } = splitOthers(Array.from({ length: 11 }, (_, i) => sp(i + 1)));
    expect(own.map((s) => s.id)).toEqual([1, 2, 3, 4, 5, 6, 7, 8]);
    expect(others.map((s) => s.id)).toEqual([9, 10, 11]);
  });
  it("keeps what the core already put in Others, and leaves out non-persons", () => {
    const { own, others } = splitOthers([sp(1), sp(2, { others: true, colorSlot: 0 }), sp(3, { notPerson: true }), sp(4)]);
    expect(own.map((s) => s.id)).toEqual([1, 4]);
    expect(others.map((s) => s.id)).toEqual([2]);
  });
});

describe("liveBlocks", () => {
  const l = (speaker: number, t0Ms: number, t1Ms: number, overlap = false) => ({ speaker, t0Ms, t1Ms, overlap });
  it("one block per line when nothing is flagged, however the lines overlap", () => {
    expect(liveBlocks([l(1, 0, 5000), l(2, 1000, 3000)])).toEqual([
      { from: 0, to: 1, stacked: false },
      { from: 1, to: 2, stacked: false },
    ]);
  });
  it("stacks flagged lines with the lines they overlap, three ways too", () => {
    const lines = [l(1, 0, 5000, true), l(2, 4000, 9000, true), l(3, 8000, 12_000, true), l(1, 20_000, 21_000)];
    expect(liveBlocks(lines)).toEqual([
      { from: 0, to: 3, stacked: true },
      { from: 3, to: 4, stacked: false },
    ]);
  });
  it("a long turn with a flagged mm-hm stacks only the turn it interrupts", () => {
    const lines = [l(1, 0, 20_000), l(1, 20_000, 40_000), l(2, 25_000, 26_000, true), l(1, 40_000, 60_000)];
    expect(liveBlocks(lines).map((b) => [b.from, b.to, b.stacked])).toEqual([
      [0, 1, false],
      [1, 3, true],
      [3, 4, false],
    ]);
  });
  it("does not stack the same speaker", () => {
    expect(liveBlocks([l(1, 0, 5000, true), l(1, 2000, 6000, true)]).every((b) => !b.stacked)).toBe(true);
  });
});

describe("parseNoteLine", () => {
  it("reads bullets and inline emphasis as data", () => {
    expect(parseNoteLine("- ship **beta** _soon_")).toEqual({
      bullet: true,
      parts: [{ text: "ship " }, { text: "beta", bold: true }, { text: " " }, { text: "soon", italic: true }],
    });
  });
  it("keeps markup-looking text as plain text", () => {
    expect(parseNoteLine("<img src=x onerror=alert(1)>")).toEqual({ bullet: false, parts: [{ text: "<img src=x onerror=alert(1)>" }] });
  });
});

it("estimates minutes of space", () => {
  expect(minutesLeft(240_000 * 90)).toBe(90);
});

it("formats bytes with the locale's decimal separator", () => {
  expect(formatBytes(1_500_000_000, "en")).toBe("1.5 GB");
  expect(formatBytes(1_500_000_000, "vi")).toBe("1,5 GB");
  expect(formatBytes(480_400_000, "en")).toBe("480 MB");
});
