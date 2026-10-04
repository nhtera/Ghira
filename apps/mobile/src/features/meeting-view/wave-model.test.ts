// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { MIN_BAR, waveBars } from "./wave-model";

const speakers = [
  { gid: "a", colorSlot: 1 },
  { gid: "b", colorSlot: 2 },
];

describe("waveBars", () => {
  it("takes the loudest 100 ms in a bar and the colour of who spoke at its middle", () => {
    // 2 s at 10 per second, 4 bars of 0.5 s.
    const peaks = Array.from({ length: 20 }, (_, i) => (i === 7 ? 255 : 0));
    const bars = waveBars({ perSecond: 10, peaks }, 2000, [
      { speakerGid: "a", t0Ms: 0, t1Ms: 1000 },
      { speakerGid: "b", t0Ms: 1000, t1Ms: 2000 },
    ], speakers, 4);
    expect(bars.map((b) => b.slot)).toEqual([1, 1, 2, 2]);
    expect(bars.map((b) => b.height)).toEqual([MIN_BAR, 1, MIN_BAR, MIN_BAR]);
  });

  it("draws neutral still bars while there is no waveform", () => {
    const bars = waveBars(null, 60_000, [], speakers, 3);
    expect(bars).toHaveLength(3);
    expect(new Set(bars.map((b) => b.height)).size).toBe(1);
    expect(bars.every((b) => b.slot === 0)).toBe(true);
  });
});
