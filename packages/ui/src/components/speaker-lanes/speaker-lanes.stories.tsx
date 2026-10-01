// SPDX-License-Identifier: Apache-2.0
import { useTranslation } from "react-i18next";
import type { Story, StoryMeta } from "../../story";
import { SpeakerLanes, type LaneSegment, type LaneSpeaker } from "./speaker-lanes";

export default { title: "Speaker lanes", width: 520 } satisfies StoryMeta;

const MIN = 60_000;
const seg = (speaker: number, a: number, b: number): LaneSegment => ({ speaker, t0Ms: a * MIN, t1Ms: b * MIN });

const ME: LaneSpeaker = { id: 0, label: "Me", colorSlot: 1 };
const LINH: LaneSpeaker = { id: 1, label: "Linh", colorSlot: 2 };
const MINH: LaneSpeaker = { id: 2, label: "Minh", colorSlot: 4 };
const SARAH: LaneSpeaker = { id: 3, label: "Sarah", colorSlot: 8 };

export const LiveGrowing: Story = {
  note: "The newest segment fades into the edge; `durationMs` is “now”.",
  render: () => (
    <SpeakerLanes live speakers={[ME, LINH]} durationMs={12 * MIN} segments={[seg(0, 0.2, 1.4), seg(0, 4.8, 5.8), seg(1, 1.7, 4.1), seg(1, 8.4, 12)]} />
  ),
};
export const StaticAfterMeeting: Story = {
  render: () => (
    <SpeakerLanes
      speakers={[ME, LINH, MINH]}
      durationMs={10 * MIN}
      segments={[seg(0, 0.2, 1.2), seg(0, 4, 4.8), seg(0, 8, 8.6), seg(1, 1.4, 4), seg(1, 5.8, 7), seg(2, 3, 3.8), seg(2, 6.6, 7.6)]}
    />
  ),
};
export const OverlapRegion: Story = {
  note: "Hatched where two people talk at once.",
  render: () => <SpeakerLanes speakers={[ME, SARAH]} durationMs={10 * MIN} segments={[seg(0, 1, 5), seg(3, 3.6, 7.6)]} />,
};

function OthersLaneDemo() {
  const { t } = useTranslation();
  const others: LaneSpeaker = { id: 99, label: `${t("detail.others")} · 3`, colorSlot: 0 };
  return (
    <SpeakerLanes
      speakers={[{ id: 3, label: "Speaker 8", colorSlot: 8 }, others]}
      durationMs={10 * MIN}
      segments={[seg(3, 2, 3), seg(3, 6, 6.8), seg(99, 0.8, 1.2), seg(99, 3, 3.6), seg(99, 5.2, 5.5), seg(99, 8, 8.5)]}
    />
  );
}
export const OthersLane: Story = { render: () => <OthersLaneDemo />, note: "Speakers beyond eight share one neutral lane." };

export const HoverScrubbing: Story = {
  note: "Marker forced here. Move the pointer over the lanes; or Tab to the timeline and use the arrow keys (Shift = 30 s, Home/End).",
  render: () => <SpeakerLanes speakers={[ME, MINH]} durationMs={10 * MIN} segments={[seg(0, 0.2, 1.4), seg(0, 4, 4.8), seg(2, 4.4, 5.6)]} onSeek={() => {}} scrubMs={4.8 * MIN} />,
};
