// SPDX-License-Identifier: Apache-2.0
// Who is in the call, in the order they arrived, plus the collapsible lanes
// (a row per speaker over the meeting time, growing live).
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Icon, SpeakerChip, SpeakerLanes } from "@ghi/ui";
import type { SpeakerInfo } from "../../bindings";
import { useShallow } from "zustand/react/shallow";
import { elapsedMs, useLive } from "../../state/live";
import { useSpeakerLabel, speakerNumber } from "../../state/speaker-label";
import { useNow } from "./clock";
import { laneModel } from "./logic";

const chipState = (s: SpeakerInfo) => (s.provisional ? "identifying" : speakerNumber(s) ? "numbered" : "named");

export function SpeakerStrip({ defaultOpen = true }: { defaultOpen?: boolean }) {
  const { t } = useTranslation();
  const speakers = useLive((s) => s.speakers);
  const lines = useLive((s) => s.lines);
  const recording = useLive((s) => s.state === "recording");
  const labelOf = useSpeakerLabel();
  const [open, setOpen] = useState(defaultOpen);
  const list = useMemo(() => Object.values(speakers).filter((s) => !s.notPerson).sort((a, b) => a.id - b.id), [speakers]);
  const others = t("detail.others");
  const { lanes, segments } = useMemo(() => laneModel(list, lines, labelOf, others), [list, lines, labelOf, others]);
  // Re-render each second so the lanes grow while recording.
  const now = useNow(recording && open);
  const clock = useLive(useShallow((s) => ({ startedAtMs: s.startedAtMs, pausedAtMs: s.pausedAtMs, pausedTotalMs: s.pausedTotalMs })));
  if (list.length === 0) return <p className="text-small m-0 text-muted">{t("speakers.empty")}</p>;
  const duration = Math.max(elapsedMs(clock, now), lines.at(-1)?.t1Ms ?? 0);
  return (
    <div data-testid="speaker-strip" className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <ul aria-label={t("speakers.title")} className="m-0 flex min-w-0 flex-1 list-none flex-wrap gap-1.5 p-0">
          {list.map((s) => (
            <li key={s.id}>
              <SpeakerChip state={chipState(s)} name={labelOf(s)} colorSlot={s.colorSlot} isMe={s.isMe} />
            </li>
          ))}
        </ul>
        <button type="button" aria-expanded={open} aria-controls="live-lanes" onClick={() => setOpen((o) => !o)} className="text-small inline-flex h-7 flex-none items-center gap-1 rounded-seg px-2 text-muted hover:bg-sunk">
          <Icon name="view_timeline" size={15} />
          {t("live.timeline")}
          <Icon name={open ? "expand_less" : "expand_more"} size={15} />
        </button>
      </div>
      {open && (
        <div id="live-lanes">
          <SpeakerLanes speakers={lanes} segments={segments} durationMs={duration} live={recording} />
        </div>
      )}
    </div>
  );
}
