// SPDX-License-Identifier: Apache-2.0
// Pure helpers for the meetings list: day groups, durations and the flat row
// model the virtualizer renders.
import type { MeetingRow } from "../../bindings";

export type DayKey = "today" | "yesterday" | { date: number };

export type ListItem =
  | { type: "header"; id: string; day: DayKey }
  | { type: "meeting"; id: string; row: MeetingRow };

const startOfDay = (ms: number) => {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
};

/** Today / Yesterday, else the day itself (local midnight, ms). */
export function dayOf(startedAt: number | null, now: number): DayKey {
  if (startedAt === null) return { date: 0 };
  const day = startOfDay(startedAt);
  const today = startOfDay(now);
  if (day === today) return "today";
  const y = new Date(today);
  y.setDate(y.getDate() - 1);
  if (day === y.getTime()) return "yesterday";
  return { date: day };
}

const sameDay = (a: DayKey, b: DayKey) =>
  typeof a === "string" || typeof b === "string" ? a === b : a.date === b.date;

/** Rows (newest first, as the core sends them) with a header before each new day. */
export function withDayHeaders(rows: MeetingRow[], now: number): ListItem[] {
  const out: ListItem[] = [];
  let last: DayKey | null = null;
  for (const row of rows) {
    const day = dayOf(row.startedAt, now);
    if (!last || !sameDay(last, day)) {
      out.push({
        type: "header",
        id: `h-${typeof day === "string" ? day : day.date}`,
        day,
      });
      last = day;
    }
    out.push({ type: "meeting", id: row.gid, row });
  }
  return out;
}

/** Whole minutes, at least 1; null for an unknown length. */
export function minutesOf(durationMs: number | null): number | null {
  return durationMs === null
    ? null
    : Math.max(0, Math.round(durationMs / 60_000));
}

/** Pull-to-refresh: the pulled distance (px) for a finger drag of `dy` (it resists), and whether releasing now refreshes. */
export function pullState(
  dy: number,
  max = 96,
): { pull: number; ready: boolean } {
  const pull = Math.min(Math.max(dy, 0) * 0.5, max);
  return { pull, ready: pull >= 48 };
}
