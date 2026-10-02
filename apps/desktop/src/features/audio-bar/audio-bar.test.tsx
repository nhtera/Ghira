// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { usePlayer } from "../../state/player";
import { carryOut, readTime } from "./playback";
import { WaveformSlider } from "./waveform";
import { columnSlots, reducePeaks, timeAt } from "./waveform-math";

afterEach(cleanup);

const spans = [
  { t0Ms: 0, t1Ms: 10_000 },
  { t0Ms: 20_000, t1Ms: 30_000 },
  { t0Ms: 31_000, t1Ms: 40_000 },
];

describe("readTime (skip silence)", () => {
  it("reports the element's time as it is when not skipping", () => {
    const audio = { currentTime: 15 };
    expect(readTime(audio, spans, false)).toBe(15_000);
    expect(audio.currentTime).toBe(15);
  });

  it("jumps over a long gap to the next speech", () => {
    const audio = { currentTime: 12 };
    expect(readTime(audio, spans, true)).toBe(20_000);
    expect(audio.currentTime).toBe(20);
  });

  it("leaves short gaps and speech alone", () => {
    const a = { currentTime: 30.5 };
    expect(readTime(a, spans, true)).toBe(30_500);
    const b = { currentTime: 5 };
    expect(readTime(b, spans, true)).toBe(5000);
    expect(b.currentTime).toBe(5);
  });
});

describe("carryOut", () => {
  it("seeks and plays, or seeks and pauses", () => {
    const audio = { currentTime: 0, pause: vi.fn(), play: vi.fn(() => Promise.resolve()) };
    carryOut(audio, { ms: 4000, play: true, n: 1 }, vi.fn());
    expect(audio.currentTime).toBe(4);
    expect(audio.play).toHaveBeenCalled();
    carryOut(audio, { ms: 6000, play: false, n: 2 }, vi.fn());
    expect(audio.pause).toHaveBeenCalled();
  });

  it("says so when playing is refused", async () => {
    const refused = vi.fn();
    carryOut({ currentTime: 0, pause: vi.fn(), play: () => Promise.reject(new Error("blocked")) }, { ms: 0, play: true, n: 3 }, refused);
    await Promise.resolve();
    await Promise.resolve();
    expect(refused).toHaveBeenCalled();
  });
});

describe("waveform math", () => {
  it("keeps the loudest bucket of each column", () => {
    // 10 buckets of 100 ms = 1 s, in 2 columns.
    const peaks = [1, 9, 3, 2, 2, 7, 0, 0, 0, 4];
    expect([...reducePeaks(peaks, 10, 1000, 2)]).toEqual([9, 7]);
    expect([...reducePeaks(peaks, 10, 0, 2)]).toEqual([0, 0]);
  });

  it("colors a column by who speaks in its middle", () => {
    const segs = [
      { gid: "1", speakerGid: "a", t0Ms: 0, t1Ms: 4000 },
      { gid: "2", speakerGid: "b", t0Ms: 6000, t1Ms: 10_000 },
    ] as never;
    const speakers = [
      { gid: "a", colorSlot: 3 },
      { gid: "b", colorSlot: 5 },
    ] as never;
    expect([...columnSlots(segs, speakers, 10_000, 5)]).toEqual([3, 3, 0, 5, 5]);
  });

  it("maps x to time within bounds", () => {
    expect(timeAt(50, 100, 60_000)).toBe(30_000);
    expect(timeAt(-5, 100, 60_000)).toBe(0);
    expect(timeAt(500, 100, 60_000)).toBe(60_000);
  });
});

describe("waveform slider", () => {
  beforeEach(() => usePlayer.setState({ currentMs: 20_000, playing: false, seekRequest: null }));

  const slider = () => render(<WaveformSlider data={{ perSecond: 10, peaks: [] }} segments={[]} speakers={[]} durationMs={60_000} seekTo={(ms) => usePlayer.getState().seek(ms)} />);

  it("is a slider with the time as its value", () => {
    slider();
    const s = screen.getByRole("slider", { name: "Playback position" });
    expect(s.getAttribute("aria-valuetext")).toBe("0:20 / 1:00");
    expect(s.getAttribute("aria-valuemax")).toBe("60");
  });

  it("moves 5 s with the arrows and to the ends with Home/End", () => {
    slider();
    const s = screen.getByRole("slider");
    fireEvent.keyDown(s, { key: "ArrowRight" });
    expect(usePlayer.getState().seekRequest?.ms).toBe(25_000);
    usePlayer.setState({ currentMs: 25_000 });
    fireEvent.keyDown(s, { key: "ArrowLeft" });
    expect(usePlayer.getState().seekRequest?.ms).toBe(20_000);
    fireEvent.keyDown(s, { key: "End" });
    expect(usePlayer.getState().seekRequest?.ms).toBe(60_000);
    fireEvent.keyDown(s, { key: "Home" });
    expect(usePlayer.getState().seekRequest?.ms).toBe(0);
  });
});
