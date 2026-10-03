// SPDX-License-Identifier: Apache-2.0
import type { Story, StoryMeta } from "../../story";
import { PrivacyIndicator, type PrivacyState } from "./privacy-indicator";

export default { title: "Privacy indicator (iOS)", platform: "ios" } satisfies StoryMeta;

const STATES: PrivacyState[] = ["local", "recording", "paused", "cloudMeeting", "cloudToday"];

export const Compact: Story = {
  render: () => (
    <ul className="m-0 flex list-none flex-col items-start gap-3 p-0">
      {STATES.map((state) => (
        <li key={state}>
          <PrivacyIndicator state={state} />
        </li>
      ))}
    </ul>
  ),
};
