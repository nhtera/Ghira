// SPDX-License-Identifier: Apache-2.0
// First-launch bookkeeping (M1): which steps exist, where to resume, and
// whether the app still needs onboarding.
import type { OnboardingState, OnboardingStep } from "../../bindings";
import { ipc } from "../../ipc";

/**
 * The steps in order. `pair` is phase 15 (QR pairing with a computer): it has no
 * screen yet, so it never shows, whatever `syncAvailable` says. The optional
 * voice step comes after the models: its speaker model downloads there.
 */
export const STEPS: readonly OnboardingStep[] = [
  "languages",
  "micPriming",
  "consent",
  "processing",
  "models",
  "voice",
  "done",
];

/** The first step not completed yet; the last one when all are. */
export function resumeStep(
  state: Pick<OnboardingState, "completed">,
): OnboardingStep {
  return STEPS.find((s) => !state.completed.includes(s)) ?? "done";
}

/** The store is not open yet (locked, still starting): the lock gate or a retry sorts it out. */
export const isTransient = (error: string) => /lock|start/i.test(error);

/**
 * `needed`: first launch. `done`: been through it. `transient`: the store is
 * locked or starting, so say nothing yet. `error`: the state could not be read
 * (the onboarding screen shows the error rather than skipping M1 silently).
 */
export async function onboardingNeed(): Promise<"needed" | "done" | "transient" | "error"> {
  const r = await ipc.commands.onboardingState().catch(() => null);
  if (r === null) return "error";
  if (r.status === "ok") return r.data.completed.includes("done") ? "done" : "needed";
  return isTransient(r.error) ? "transient" : "error";
}

/** True until the user has been through the last step; a read error counts as needed (it is shown there). */
export async function needsOnboarding(): Promise<boolean> {
  const need = await onboardingNeed();
  return need === "needed" || need === "error";
}
