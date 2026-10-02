// SPDX-License-Identifier: Apache-2.0
// How a stored speaker is shown on the detail screen: the user's name, "Me",
// or "Speaker N" (localized), with the initial for the avatar.
import type { TFunction } from "i18next";
import type { MeetingSpeaker } from "../../bindings";

export type SpeakerDisplay = {
  name: string;
  initial?: string;
  colorSlot: number;
  isMe: boolean;
  named: boolean;
};

export function speakerDisplay(
  s: MeetingSpeaker,
  t: TFunction,
): SpeakerDisplay {
  const named = !!s.name && !s.isMe;
  const name = s.isMe
    ? t("speakers.me")
    : (s.name ?? t("speakers.numbered", { number: s.number }));
  return {
    name,
    initial: s.name || s.isMe ? undefined : String(s.number),
    colorSlot: s.colorSlot,
    isMe: s.isMe,
    named,
  };
}

export const findSpeaker = (
  speakers: readonly MeetingSpeaker[],
  gid: string | null | undefined,
) => (gid ? speakers.find((s) => s.gid === gid) : undefined);
