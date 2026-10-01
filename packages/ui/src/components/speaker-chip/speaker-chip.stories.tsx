// SPDX-License-Identifier: Apache-2.0
import { useState } from "react";
import type { Story, StoryMeta } from "../../story";
import { SpeakerChip } from "./speaker-chip";

export default { title: "Speaker chip", width: 300 } satisfies StoryMeta;

export const UnknownIdentifying: Story = { render: () => <SpeakerChip state="identifying" /> };
export const SpeakerN: Story = { render: () => <SpeakerChip state="numbered" name="Speaker 2" colorSlot={2} /> };
export const Suggested: Story = { render: () => <SpeakerChip state="suggested" name="Speaker 2" colorSlot={2} suggestion="Linh" /> };
export const Named: Story = { render: () => <SpeakerChip state="named" name="Sarah" colorSlot={8} /> };
export const AutoNamedVoice: Story = { render: () => <SpeakerChip state="auto" name="Minh" colorSlot={4} /> };
export const Merged: Story = { render: () => <SpeakerChip state="merged" name="Linh" mergedFrom="Speaker 5" colorSlot={2} /> };
export const Selected: Story = { render: () => <SpeakerChip state="named" name="Linh" colorSlot={2} selected /> };
export const MeInCallMode: Story = { render: () => <SpeakerChip state="named" name="Me" isMe colorSlot={1} /> };

function Toggle() {
  const [on, setOn] = useState(false);
  return <SpeakerChip state="named" name="Đặng Thu Hà" colorSlot={5} selected={on} onClick={() => setOn(!on)} />;
}
export const Interactive: Story = { render: () => <Toggle />, note: "Click to select (aria-pressed); Vietnamese name." };
