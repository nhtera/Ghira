// SPDX-License-Identifier: Apache-2.0
import type { Story, StoryMeta } from "../../story";
import { CitationChip } from "./citation-chip";

export default { title: "Citation chip", width: 320 } satisfies StoryMeta;

const MOMENT = 572_000; // 09:32

export const Default: Story = { render: () => <CitationChip timeMs={MOMENT} /> };
export const Hover: Story = { render: () => <CitationChip timeMs={MOMENT} className="border-accent! text-accent!" />, note: "Forced for the contact sheet; real hover is CSS." };
export const Focus: Story = {
  render: () => <CitationChip timeMs={MOMENT} className="border-accent! text-accent! outline-2 outline-offset-2 outline-accent" />,
  note: "Forced; real focus uses the global focus ring.",
};
export const Visited: Story = { render: () => <CitationChip timeMs={MOMENT} visited /> };
export const BrokenSource: Story = { render: () => <CitationChip timeMs={MOMENT} broken text="Tue" />, note: "Audio deleted; dashed." };
export const SourceNumber: Story = { render: () => <CitationChip index={3} /> };
