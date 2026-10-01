// SPDX-License-Identifier: Apache-2.0
import type { ProcessingStep } from "@ghi/ui";
import { STAGES, stageOf, type Processing } from "./processing-store";

/** Steps before the current stage are done; no stage yet = the first one runs. */
export function stepsFor(p: Pick<Processing, "stage" | "kind">): ProcessingStep[] {
  const at = Math.max(0, STAGES.indexOf(stageOf(p.kind, p.stage) ?? "decoding"));
  return STAGES.map((id, i) => ({ id, status: i < at ? "done" : i === at ? "running" : "pending" }));
}
