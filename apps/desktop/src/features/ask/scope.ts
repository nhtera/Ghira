// SPDX-License-Identifier: Apache-2.0
// Where "Ask" looks: all meetings, one meeting, or a date range. Ranges are
// calendar days like the library's date filter; counts come from the loaded
// library rows.
import type { AskScope, MeetingRow } from "../../bindings";
import { dateFrom } from "../library/filters";

export type ScopeKind = "all" | "meeting" | "person" | "range";
export type RangeKey = "last7Days" | "last30Days" | "thisYear";
export const RANGES: readonly RangeKey[] = ["last7Days", "last30Days", "thisYear"];

export function rangeFrom(key: RangeKey, now: Date): number {
  if (key === "last7Days") return dateFrom("week", now);
  if (key === "last30Days") return dateFrom("month", now);
  return new Date(now.getFullYear(), 0, 1).getTime();
}

/** Meetings the core can read: finished ones (not recording, being processed or failed). */
export const searchable = (r: Pick<MeetingRow, "status" | "job">) => r.status !== "recording" && r.status !== "processing" && r.status !== "failed" && !r.job;

export const countInRange = (rows: readonly Pick<MeetingRow, "startedAt" | "status" | "job">[], key: RangeKey, now: Date) => {
  const from = rangeFrom(key, now);
  return rows.filter((r) => searchable(r) && r.startedAt != null && r.startedAt >= from).length;
};

export function buildScope(kind: ScopeKind, meeting: string | undefined, range: RangeKey, now: Date, person?: string): AskScope {
  return {
    meetings: kind === "meeting" && meeting ? [meeting] : [],
    fromMs: kind === "range" ? rangeFrom(range, now) : null,
    toMs: null,
    persons: kind === "person" && person ? [person] : [],
  };
}
