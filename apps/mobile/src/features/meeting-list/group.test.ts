// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import type { MeetingRow } from "../../bindings";
import { dayOf, minutesOf, pullState, withDayHeaders } from "./group";

const row = (gid: string, startedAt: number | null): MeetingRow =>
  ({ gid, startedAt }) as MeetingRow;
const NOW = new Date(2026, 9, 4, 12, 0).getTime();
const at = (m: number, d: number, h = 9) => new Date(2026, m, d, h).getTime();

describe("dayOf", () => {
  it("names today and yesterday and keeps older days as dates", () => {
    expect(dayOf(at(9, 4, 0), NOW)).toBe("today");
    expect(dayOf(at(9, 3, 23), NOW)).toBe("yesterday");
    expect(dayOf(at(8, 12), NOW)).toEqual({
      date: new Date(2026, 8, 12).getTime(),
    });
  });

  it("puts a meeting without a start time in its own bucket", () => {
    expect(dayOf(null, NOW)).toEqual({ date: 0 });
  });
});

describe("withDayHeaders", () => {
  it("adds one header per day, keeping the order", () => {
    const items = withDayHeaders(
      [
        row("a", at(9, 4, 11)),
        row("b", at(9, 4, 9)),
        row("c", at(9, 3, 15)),
        row("d", at(8, 12)),
      ],
      NOW,
    );
    expect(
      items.map((i) =>
        i.type === "header"
          ? `#${typeof i.day === "string" ? i.day : "date"}`
          : i.id,
      ),
    ).toEqual(["#today", "a", "b", "#yesterday", "c", "#date", "d"]);
  });

  it("is empty for no meetings", () => {
    expect(withDayHeaders([], NOW)).toEqual([]);
  });
});

describe("minutesOf", () => {
  it("rounds to minutes", () => {
    expect(minutesOf(42 * 60_000)).toBe(42);
    expect(minutesOf(20_000)).toBe(0);
    expect(minutesOf(null)).toBeNull();
  });
});

describe("pullState", () => {
  it("resists the finger and arms past the threshold", () => {
    expect(pullState(-10)).toEqual({ pull: 0, ready: false });
    expect(pullState(40).ready).toBe(false);
    expect(pullState(200)).toEqual({ pull: 96, ready: true });
  });
});
