// SPDX-License-Identifier: Apache-2.0
import type { Story, StoryMeta } from "../../story";
import { LevelMeter } from "./level-meter";

export default { title: "Level meter", width: 420 } satisfies StoryMeta;

export const Silent: Story = { render: () => <LevelMeter source="mic" db={-58} /> };
export const Normal: Story = { render: () => <LevelMeter source="mic" db={-22} /> };
export const Clipping: Story = { render: () => <LevelMeter source="mic" db={-0.3} /> };
export const NoDevice: Story = { render: () => <LevelMeter source="mic" db={null} /> };
export const System: Story = {
  render: () => (
    <div className="flex flex-col gap-2">
      <LevelMeter source="system" db={-30} />
      <LevelMeter source="system" db={null} />
    </div>
  ),
};
export const Footer: Story = {
  render: () => (
    <div className="flex items-center gap-4">
      <LevelMeter source="mic" db={-22} compact />
      <LevelMeter source="system" db={-58} compact />
    </div>
  ),
  note: "The live footer form: icon, name and a short bar.",
};
