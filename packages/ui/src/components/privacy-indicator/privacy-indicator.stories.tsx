// SPDX-License-Identifier: Apache-2.0
import type { Story, StoryMeta } from "../../story";
import { PrivacyIndicator } from "./privacy-indicator";

export default { title: "Privacy indicator", width: 300 } satisfies StoryMeta;

export const LocalOnly: Story = { render: () => <PrivacyIndicator state="local" /> };
export const Recording: Story = { render: () => <PrivacyIndicator state="recording" /> };
export const Paused: Story = { render: () => <PrivacyIndicator state="paused" /> };
export const CloudUsedThisMeeting: Story = { render: () => <PrivacyIndicator state="cloudMeeting" /> };
export const CloudUsedToday: Story = { render: () => <PrivacyIndicator state="cloudToday" /> };
