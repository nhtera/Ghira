// SPDX-License-Identifier: Apache-2.0
import type { Story, StoryMeta } from "../../story";
import { StatusPill, type StatusKind } from "./status-pill";

export default { title: "Status pill (iOS)", platform: "ios" } satisfies StoryMeta;

const KINDS: StatusKind[] = ["ready", "processing", "finalPass", "needsNames", "cloudEnhanced", "failed", "waitingSync", "waitingModels", "recording"];

export const AllStatuses: Story = {
  render: () => (
    <ul className="m-0 flex list-none flex-col items-start gap-3 p-0">
      {KINDS.map((status) => (
        <li key={status}>
          <StatusPill status={status} percent={62} onRetry={status === "failed" ? () => {} : undefined} />
        </li>
      ))}
    </ul>
  ),
};
