// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { talkShare } from "./talk-share";

describe("talkShare", () => {
  const spk = [
    { gid: "a", lines: 2 },
    { gid: "b", lines: 1 },
  ];
  const seg = (speakerGid: string | null, t0Ms: number | null, t1Ms: number | null) => ({ speakerGid, t0Ms, t1Ms });

  it("sums talk time per speaker, biggest first, with turns from the speakers", () => {
    const r = talkShare([seg("a", 0, 3000), seg("b", 3000, 4000), seg("a", 4000, 7000)], spk);
    expect(r).toEqual([
      { gid: "a", talkMs: 6000, pct: 86, turns: 2 },
      { gid: "b", talkMs: 1000, pct: 14, turns: 1 },
    ]);
  });

  it("always adds up to 100 (largest remainder)", () => {
    const r = talkShare([seg("a", 0, 1000), seg("b", 0, 1000), seg("c", 0, 1000)], [...spk, { gid: "c", lines: 1 }]);
    expect(r.map((e) => e.pct).sort()).toEqual([33, 33, 34]);
    for (const n of [3, 7, 11]) {
      const many = Array.from({ length: n }, (_, i) => seg(`s${i}`, 0, 1000 + i * 137));
      const total = talkShare(many, many.map((m) => ({ gid: m.speakerGid!, lines: 1 }))).reduce((a, e) => a + e.pct, 0);
      expect(total).toBe(100);
    }
  });

  it("puts null and unknown speakers under unassigned, with their line count as turns", () => {
    const r = talkShare([seg("a", 0, 2000), seg(null, 2000, 3000), seg("gone", 3000, 4000)], spk);
    expect(r.find((e) => e.gid === null)).toEqual({ gid: null, talkMs: 2000, pct: 50, turns: 2 });
    expect(r.find((e) => e.gid === "a")?.pct).toBe(50);
  });

  it("ignores zero-length and missing-time lines; nothing to share is empty", () => {
    expect(talkShare([seg("a", 5000, 5000), seg("b", null, null), seg("a", 9000, 8000)], spk)).toEqual([]);
    expect(talkShare([], spk)).toEqual([]);
    const r = talkShare([seg("a", 0, 1000), seg("b", 1000, 1000)], spk);
    expect(r).toEqual([{ gid: "a", talkMs: 1000, pct: 100, turns: 2 }]);
  });
});
