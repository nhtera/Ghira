// SPDX-License-Identifier: Apache-2.0
import type { Story, StoryMeta } from "../../story";
import { SyncChip, type SyncChipKind } from "./sync-chip";

export default { title: "Sync chip (iOS)", platform: "ios" } satisfies StoryMeta;

const KINDS: SyncChipKind[] = [
  { kind: "recorded" },
  { kind: "processingOnPhone", percent: 62 },
  { kind: "processedOnPhone" },
  { kind: "waitingForModels" },
  { kind: "failed" },
  { kind: "synced" },
  { kind: "waitingForWifi" },
  { kind: "waitingForComputer" },
  { kind: "finalOnDesktop", percent: 62 },
];

export const AllKinds: Story = {
  render: () => (
    <ul className="m-0 flex list-none flex-col items-start gap-3 p-0">
      {KINDS.map((chip) => (
        <li key={chip.kind}>
          <SyncChip chip={chip} device="MacBook Pro" />
        </li>
      ))}
    </ul>
  ),
};

export const FailedRetry: Story = { render: () => <SyncChip chip={{ kind: "failed" }} onRetry={() => {}} />, note: "A button (44 pt hit area) when a retry is possible." };
