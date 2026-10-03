// SPDX-License-Identifier: Apache-2.0
import { sampleLine } from "../avatar/sample-data";
import type { Story, StoryMeta } from "../../story";
import { TranscriptLine, wordsFromText, type TranscriptLineProps } from "./transcript-line";

export default { title: "Transcript line", width: 520 } satisfies StoryMeta;

function Line({ i, ...rest }: { i: number } & Partial<TranscriptLineProps>) {
  const l = sampleLine(i);
  return <TranscriptLine startMs={l.startMs} speaker={l.speaker} words={wordsFromText(l.text, l.low)} {...rest} />;
}

export const Partial: Story = {
  render: () => <Line i={12} startMs={2470_000} partial words={wordsFromText("Tốt. Chốt lại: live rename, voice profile có consent")} />,
  note: "Muted; the newest words pulse (still under reduced motion).",
};
export const Final: Story = { render: () => <Line i={3} /> };
export const ProvisionalSpeaker: Story = {
  render: () => <TranscriptLine startMs={286_000} speaker={null} words={wordsFromText("Em đã gom feedback từ 12 user test.")} />,
};
export const LowConfidence: Story = { render: () => <Line i={2} />, note: "Dotted underline on flagged words; screen readers hear “Low confidence”." };
export const Marked: Story = { render: () => <Line i={7} marked /> };
export const PlayingKaraoke: Story = {
  render: () => <Line i={4} playing activeWordIndex={2} />,
  note: "Only the current word is highlighted.",
};
export const Overlap: Story = {
  render: () => <Line i={6} overlap />,
  note: "Talking over each other: marker with a hint, text muted like a low-confidence line.",
};
export const Selected: Story = { render: () => <Line i={5} selected /> };
export const Edited: Story = { render: () => <Line i={10} edited /> };
export const HoverActions: Story = {
  render: () => <Line i={5} onEdit={() => {}} onChangeSpeaker={() => {}} onPlay={() => {}} className="[&>div:last-child]:opacity-100" />,
  note: "Edit text and Change speaker appear on hover or focus-within (forced here).",
};
export const Small: Story = {
  render: () => <Line i={5} small />,
  note: "Narrow column of the live Focus layout.",
};
