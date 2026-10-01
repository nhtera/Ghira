// SPDX-License-Identifier: Apache-2.0
// Which StatusPill a library row shows. Live job progress (core events) beats
// the progress the last list_meetings call saw.
import type { StatusKind } from "@ghi/ui";
import type { MeetingRow } from "../../bindings";

export type RowStatus = { status: StatusKind; percent?: number };

export function rowStatus(row: MeetingRow, liveProgress: number | undefined, needsNames: boolean): RowStatus {
  if (row.status === "recording") return { status: "recording" };
  if (row.status === "failed") return { status: "failed" };
  if (row.job?.waitingForModels) return { status: "waitingModels" };
  if (row.status === "processing" || row.job) {
    const p = liveProgress ?? row.job?.progress ?? undefined;
    return { status: "processing", percent: p == null ? undefined : p * 100 };
  }
  if (needsNames) return { status: "needsNames" };
  return { status: row.cloudUsed ? "cloudEnhanced" : "ready" };
}
