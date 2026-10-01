// SPDX-License-Identifier: Apache-2.0
import type { Story, StoryMeta } from "../../story";
import { ProcessingStepper, type ProcessingStep } from "./processing-stepper";

export default { title: "Processing stepper", width: 420 } satisfies StoryMeta;

const IDS = ["refiningSpeakers", "matchingVoices", "improvingTranscript", "writingNotes"] as const;
const make = (...s: Array<ProcessingStep["status"] | [ProcessingStep["status"], number, number]>): ProcessingStep[] =>
  IDS.map((id, i) => {
    const v = s[i]!;
    return Array.isArray(v) ? { id, status: v[0], progress: v[1], estimateSeconds: v[2] } : { id, status: v };
  });

export const Pending: Story = { render: () => <ProcessingStepper steps={make("pending", "pending", "pending", "pending")} /> };
export const Running: Story = {
  render: () => <ProcessingStepper steps={make("done", "skipped", ["running", 45, 40], "pending")} />,
  note: "Done, skipped, running with progress and estimate, pending.",
};
export const RunningMinutes: Story = { render: () => <ProcessingStepper steps={make("done", "done", "done", ["running", 20, 210])} /> };
export const WithDecoding: Story = {
  render: () => (
    <ProcessingStepper
      steps={[
        { id: "decoding", status: "running", progress: 70, estimateSeconds: 20 },
        ...make("pending", "pending", "pending", "pending"),
      ]}
    />
  ),
};
export const Done: Story = { render: () => <ProcessingStepper steps={make("done", "done", "done", "done")} /> };
export const Failed: Story = { render: () => <ProcessingStepper steps={make("done", "done", "done", "failed")} onRetry={() => {}} /> };
export const Skipped: Story = { render: () => <ProcessingStepper steps={make("done", "skipped", "skipped", "done")} /> };
