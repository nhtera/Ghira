// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { buildScope, countInRange, rangeFrom, searchable } from "./scope";

const NOW = new Date(2026, 9, 3, 15, 30); // Sat 3 Oct 2026, local

describe("ask scope", () => {
  it("ranges start at local midnight", () => {
    expect(rangeFrom("thisYear", NOW)).toBe(new Date(2026, 0, 1).getTime());
    expect(rangeFrom("last7Days", NOW)).toBe(new Date(2026, 9, 3 - 6).getTime());
    expect(rangeFrom("last30Days", NOW)).toBe(new Date(2026, 9, 3 - 29).getTime());
  });

  it("builds the scope the core expects", () => {
    expect(buildScope("all", "m1", "thisYear", NOW)).toEqual({ meetings: [], fromMs: null, toMs: null, persons: [] });
    expect(buildScope("meeting", "m1", "thisYear", NOW).meetings).toEqual(["m1"]);
    expect(buildScope("range", undefined, "thisYear", NOW)).toEqual({ meetings: [], fromMs: new Date(2026, 0, 1).getTime(), toMs: null, persons: [] });
  });

  it("counts only finished meetings inside the range", () => {
    const row = (startedAt: number | null, status = "ready", job = null) => ({ startedAt, status, job });
    const rows = [
      row(new Date(2026, 9, 2).getTime()),
      row(new Date(2026, 9, 2).getTime(), "recording"),
      row(new Date(2026, 9, 2).getTime(), "processing"),
      row(new Date(2026, 9, 2).getTime(), "failed"),
      row(new Date(2026, 3, 2).getTime()),
      row(null),
    ];
    expect(rows.filter(searchable)).toHaveLength(3);
    expect(countInRange(rows, "last7Days", NOW)).toBe(1);
    expect(countInRange(rows, "thisYear", NOW)).toBe(2);
  });
});
