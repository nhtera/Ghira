// SPDX-License-Identifier: Apache-2.0
// Library grouping: Today, Yesterday, weekday names this week, "Last week",
// then one group per month. Rows arrive newest first and keep that order.
import type { MeetingRow } from "../../bindings";

export type DayGroupKey =
  | { kind: "today" }
  | { kind: "yesterday" }
  | { kind: "weekday"; day: number }
  | { kind: "lastWeek" }
  | { kind: "month"; month: number; year: number }
  | { kind: "unknown" };

export type DayGroup = { id: string; key: DayGroupKey; rows: MeetingRow[] };

const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
const DAY = 86_400_000;

export function groupKeyOf(startedAt: number | null, now: Date): DayGroupKey {
  if (startedAt == null) return { kind: "unknown" };
  const d = new Date(startedAt);
  // Calendar days (not 24 h blocks), so a DST change can't shift a row.
  const ago = Math.round((startOfDay(now) - startOfDay(d)) / DAY);
  if (ago <= 0) return { kind: "today" };
  if (ago === 1) return { kind: "yesterday" };
  if (ago < 7) return { kind: "weekday", day: d.getDay() };
  if (ago < 14) return { kind: "lastWeek" };
  return { kind: "month", month: d.getMonth(), year: d.getFullYear() };
}

const idOf = (k: DayGroupKey) => (k.kind === "month" ? `m${k.year}-${k.month}` : k.kind === "weekday" ? `w${k.day}` : k.kind);

export function groupByDay(rows: MeetingRow[], now: Date = new Date()): DayGroup[] {
  const groups: DayGroup[] = [];
  for (const row of rows) {
    const key = groupKeyOf(row.startedAt, now);
    const id = idOf(key);
    const last = groups[groups.length - 1];
    if (last && last.id === id) last.rows.push(row);
    else groups.push({ id, key, rows: [row] });
  }
  return groups;
}
