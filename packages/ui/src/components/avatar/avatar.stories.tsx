// SPDX-License-Identifier: Apache-2.0
import type { ReactNode } from "react";
import type { Story, StoryMeta } from "../../story";
import { Avatar } from "./avatar";

export default { title: "Avatar", width: 360 } satisfies StoryMeta;

const SIZES = ["sm", "md", "lg", "xl"] as const;

function Row({ children }: { children: (size: (typeof SIZES)[number]) => ReactNode }) {
  return <div className="flex items-center gap-4">{SIZES.map((s) => <span key={s}>{children(s)}</span>)}</div>;
}

export const Person: Story = { render: () => <Row>{(size) => <Avatar name="Linh" colorSlot={2} size={size} />}</Row> };
export const VietnameseName: Story = {
  note: "Initial from the first word, diacritics kept.",
  render: () => (
    <div className="flex items-center gap-4">
      <Avatar name="Đặng Thu Hà" colorSlot={5} />
      <Avatar name="Ông Văn An" colorSlot={3} />
      <Avatar name="Nguyễn Minh" colorSlot={4} />
    </div>
  ),
};
export const UnknownVoice: Story = { render: () => <Row>{(size) => <Avatar kind="unknown" size={size} />}</Row> };
export const Me: Story = { render: () => <Row>{(size) => <Avatar kind="me" colorSlot={1} size={size} />}</Row> };
export const Group: Story = { render: () => <Row>{(size) => <Avatar kind="group" count={3} size={size} />}</Row> };
export const OthersNeutral: Story = { render: () => <Avatar name="Others" colorSlot={0} /> };
export const AllSpeakerColors: Story = {
  render: () => (
    <div className="flex gap-2">
      {[1, 2, 3, 4, 5, 6, 7, 8].map((n) => <Avatar key={n} name={`S${n}`} initial={String(n)} colorSlot={n} />)}
    </div>
  ),
};
