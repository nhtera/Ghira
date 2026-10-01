// SPDX-License-Identifier: Apache-2.0
// "Change speaker" on one transcript line: move it to someone else, or to a
// new speaker. The core has no "reassign one line" command, so this splits
// the line off and merges it into the chosen speaker.
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { Popover, SpeakerChip } from "@ghi/ui";
import { useLive } from "../../state/live";
import { speakerNumber, useSpeakerLabel } from "../../state/speaker-label";
import { useSpeakerActions } from "./use-speaker-actions";

export function LineSpeakerPicker({ gid, from, onClose }: { gid: string; from: number; onClose: () => void }) {
  const { t } = useTranslation();
  const actions = useSpeakerActions();
  const labelOf = useSpeakerLabel();
  const ids = useLive(useShallow((s) => Object.values(s.speakers).filter((x) => x.id !== from && !x.notPerson && !x.provisional).map((x) => x.id)));
  const speakers = useLive.getState().speakers;

  const move = async (target: number | null) => {
    const id = await actions.split(from, [gid]);
    if (id == null) return onClose();
    if (target != null) await actions.merge(id, target);
    onClose();
  };
  return (
    <Popover
      open
      onOpenChange={(o) => !o && onClose()}
      label={t("speakers.line.changeSpeaker")}
      trigger={<span aria-hidden="true" className="pointer-events-none absolute top-1 right-2 size-6" />}
      align="end"
    >
      <div className="flex w-64 flex-col gap-1.5">
        <b className="text-body font-semibold">{t("speakers.line.pickTarget")}</b>
        <ul className="m-0 flex list-none flex-col gap-0.5 p-0">
          {ids.map((id) => {
            const s = speakers[id];
            return (
              <li key={id}>
                <button type="button" onClick={() => void move(id)} className="flex h-9 w-full items-center rounded-seg px-1 hover:bg-sunk">
                  <SpeakerChip state={speakerNumber(s) ? "numbered" : "named"} name={labelOf(s)} colorSlot={s.colorSlot} isMe={s.isMe} className="border-transparent" />
                </button>
              </li>
            );
          })}
          <li>
            <button type="button" onClick={() => void move(null)} className="text-body flex h-9 w-full items-center rounded-seg px-2 hover:bg-sunk">
              {t("speakers.newPerson")}
            </button>
          </li>
        </ul>
      </div>
    </Popover>
  );
}
