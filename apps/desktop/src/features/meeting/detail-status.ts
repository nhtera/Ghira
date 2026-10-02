// SPDX-License-Identifier: Apache-2.0
// The detail header's StatusPill: same mapping as a library row, from the
// detail's own fields. Progress from the job is kept fresh by useMeetingEvents.
import type { StatusKind } from "@ghi/ui";
import type { MeetingDetail } from "../../bindings";

export function needsNames(d: Pick<MeetingDetail, "speakers">): boolean {
  return d.speakers.some((s) => !s.isMe && !s.notPerson && !s.name);
}

export function detailStatus(
  d: Pick<MeetingDetail, "status" | "job" | "cloudUsed" | "speakers">,
): { status: StatusKind; percent?: number } {
  if (d.status === "recording") return { status: "recording" };
  if (d.status === "failed") return { status: "failed" };
  if (d.job?.waitingForModels) return { status: "waitingModels" };
  if (d.status === "processing" || d.job)
    return {
      status: "processing",
      percent: d.job?.progress == null ? undefined : d.job.progress * 100,
    };
  if (needsNames(d)) return { status: "needsNames" };
  return { status: d.cloudUsed ? "cloudEnhanced" : "ready" };
}
