// SPDX-License-Identifier: Apache-2.0
import type { Story, StoryMeta } from "../../story";
import { StatusPill } from "./status-pill";

export default { title: "Status pill", width: 300 } satisfies StoryMeta;

export const Ready: Story = { render: () => <StatusPill status="ready" /> };
export const Processing: Story = { render: () => <StatusPill status="processing" percent={62} /> };
export const NeedsNames: Story = { render: () => <StatusPill status="needsNames" /> };
export const CloudEnhanced: Story = { render: () => <StatusPill status="cloudEnhanced" /> };
export const Failed: Story = { render: () => <StatusPill status="failed" onRetry={() => {}} />, note: "A button when a retry is possible." };
export const WaitingForSync: Story = { render: () => <StatusPill status="waitingSync" /> };
export const WaitingForModels: Story = { render: () => <StatusPill status="waitingModels" />, note: "Record now, process later." };
export const Recording: Story = { render: () => <StatusPill status="recording" /> };
