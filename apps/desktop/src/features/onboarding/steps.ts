// SPDX-License-Identifier: Apache-2.0
// The onboarding step machine (D1): which steps run, and where next/back go.
export const ALL_STEPS = ["welcome", "languages", "models", "permissions", "voice", "test", "recovery", "done"] as const;
export type StepId = (typeof ALL_STEPS)[number];

export const isStepId = (s: string | undefined): s is StepId => (ALL_STEPS as readonly string[]).includes(s ?? "");

/** The steps in the flow. "Your voice" is hidden until voice profiles exist (phase 14). */
export function flowSteps(voiceEnabled: boolean): StepId[] {
  return ALL_STEPS.filter((s) => s !== "voice" || voiceEnabled);
}

/** The step after `step` (the last one stays). A step not in the flow (a typed URL) moves to the next one that is. */
export function nextStep(step: StepId, voiceEnabled: boolean): StepId {
  const flow = flowSteps(voiceEnabled);
  const i = flow.indexOf(step);
  if (i >= 0) return flow[Math.min(flow.length - 1, i + 1)]!;
  return flow.find((s) => ALL_STEPS.indexOf(s) > ALL_STEPS.indexOf(step)) ?? flow[flow.length - 1]!;
}

/** The step before `step` (the first one stays). */
export function prevStep(step: StepId, voiceEnabled: boolean): StepId {
  const flow = flowSteps(voiceEnabled);
  const i = flow.indexOf(step);
  if (i >= 0) return flow[Math.max(0, i - 1)]!;
  return [...flow].reverse().find((s) => ALL_STEPS.indexOf(s) < ALL_STEPS.indexOf(step)) ?? flow[0]!;
}

/** The step to show for a URL parameter (unknown or hidden steps fall back to the nearest one in the flow). */
export function resolveStep(raw: string | undefined, voiceEnabled: boolean): StepId {
  if (!isStepId(raw)) return "welcome";
  return flowSteps(voiceEnabled).includes(raw) ? raw : nextStep(raw, voiceEnabled);
}
