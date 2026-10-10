// SPDX-License-Identifier: Apache-2.0
import type { ProcessingStep } from "@ghi/ui";
import { STAGES, stageOf, stageProgress, type Processing } from "./processing-store";

/** Steps before the current stage are done; no stage yet = the first one runs. */
export function stepsFor(p: Pick<Processing, "stage" | "kind"> & { progress?: number | null }): ProcessingStep[] {
  const at = Math.max(0, STAGES.indexOf(stageOf(p.kind, p.stage) ?? "decoding"));
  const own = stageProgress(p.kind, stageOf(p.kind, p.stage), p.progress);
  return STAGES.map((id, i) => ({
    id,
    status: i < at ? "done" : i === at ? "running" : "pending",
    // The running stage's own progress, when the core reports one.
    ...(i === at && own != null ? { progress: Math.round(own * 100) } : {}),
  }));
}

export { overallProgress } from "./processing-store";
