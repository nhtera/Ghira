// SPDX-License-Identifier: Apache-2.0
// A stored meeting's speaker chip with its voice actions: "Sounds like …"
// (accept / dismiss, from the final pass's voice match), "This is me" and
// "Not me". Stored meetings only: the live strip has its own popover. In a
// call only the mic speaker can be Me, and the core decides that on its own,
// so calls get no Me buttons at all (only the voice suggestion, if any).
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Popover, SpeakerChip } from "@ghi/ui";
import type { MeetingSpeaker } from "../../bindings";
import { speakerDisplay } from "../meeting/speaker-display";
import { useStoredSpeakerActions } from "./use-speaker-actions";

export function StoredSpeaker({ meeting, mode, speaker }: { meeting: string; mode: string; speaker: MeetingSpeaker }) {
  const { t } = useTranslation();
  const actions = useStoredSpeakerActions(meeting);
  const [open, setOpen] = useState(false);
  const d = speakerDisplay(speaker, t);
  const s = speaker.suggestion;
  const chip = <SpeakerChip state={d.named ? "named" : "numbered"} name={d.name} colorSlot={d.colorSlot} isMe={d.isMe} />;

  if (s) {
    const suggested = s.isMe ? t("speakers.me") : s.name;
    return (
      <span data-testid="voice-suggestion" className="inline-flex h-full">
        <SpeakerChip
          state="suggested"
          name={d.name}
          colorSlot={d.colorSlot}
          suggestion={suggested}
          suggestionLabel={t("speakers.soundsLikeName", { name: suggested })}
          onAcceptSuggestion={() => void actions.accept(speaker.gid, s.isMe, suggested)}
          onDismissSuggestion={() => void actions.dismiss(speaker.gid)}
        />
      </span>
    );
  }
  if (mode === "call") return chip;
  return (
    <Popover
      open={open}
      onOpenChange={setOpen}
      label={t("speakers.panel")}
      trigger={
        <button type="button" className="rounded-full">
          {chip}
        </button>
      }
    >
      <div className="flex w-64 flex-col gap-1.5">
        <b className="text-body font-semibold">{d.name}</b>
        <p className="text-small m-0 text-muted">{t("speakers.linesInMeeting", { count: speaker.lines })}</p>
        {speaker.isMe ? (
          <Button icon="person_remove" onClick={() => void actions.notMe(speaker.gid).then((ok) => ok && setOpen(false))}>
            {t("speakers.notMe")}
          </Button>
        ) : (
          <Button icon="person" onClick={() => void actions.thisIsMe(speaker.gid).then((ok) => ok && setOpen(false))}>
            {t("speakers.thisIsMe")}
          </Button>
        )}
      </div>
    </Popover>
  );
}
