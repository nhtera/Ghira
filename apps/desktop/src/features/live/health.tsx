// SPDX-License-Identifier: Apache-2.0
// Health indicator (brief D4): one quiet line that opens into rows (speech
// recognition, speaker detection, audio route, disk). Warnings also come up
// as inline banners; this is where the numbers live.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { Icon, cn, usePlatform, type IconName } from "@ghi/ui";
import { useLive } from "../../state/live";
import { LAG_WARN_S, formatBytes, minutesLeft } from "./logic";

type Row = { id: string; icon: IconName; title: string; detail: string; warn: boolean; hint?: string };

export function Health() {
  const { t, i18n } = useTranslation();
  const platform = usePlatform();
  const [open, setOpen] = useState(false);
  const h = useLive(useShallow((s) => ({ lag: s.asrLagS, aec: s.aec, hfp: s.capture.bluetoothHfp, low: s.capture.diskLowBytes, full: s.capture.diskFull, recordOnly: s.recordOnly })));
  const seconds = Math.round(h.lag * 10) / 10;
  const lagging = h.lag > LAG_WARN_S;
  const rows: Row[] = [
    { id: "asr", icon: "subtitles", title: t("live.health.transcription.title"), detail: h.recordOnly ? t("live.health.recordOnly") : t("live.health.transcription.detail", { context: platform, seconds }), warn: lagging, hint: lagging ? t("live.health.switchToFast") : undefined },
    { id: "speakers", icon: "groups", title: t("live.health.speakers.title"), detail: t("live.health.speakers.detail", { seconds }), warn: lagging },
    { id: "audio", icon: "headphones", title: t("live.health.audio.title"), detail: h.aec ? t("live.audio.speakersAecOn") : t("live.audio.headphonesAecOff"), warn: h.hfp, hint: h.hfp ? t("live.health.bluetoothHfp") : undefined },
    {
      id: "disk",
      icon: "hard_drive",
      title: t("live.health.disk.title"),
      detail: h.full ? t("live.banner.diskFull") : h.low != null ? t("system.diskLow", { free: formatBytes(h.low, i18n.language), minutes: minutesLeft(h.low) }) : t("live.health.allGood"),
      warn: h.full || h.low != null,
    },
  ];
  const issues = rows.filter((r) => r.warn).length;
  return (
    <div data-testid="health">
      <button
        type="button"
        aria-expanded={open}
        aria-controls="live-health-rows"
        onClick={() => setOpen((o) => !o)}
        className={cn("text-small inline-flex h-7 items-center gap-1.5 rounded-seg px-2 hover:bg-sunk", issues ? "font-medium text-warn" : "text-muted")}
      >
        <Icon name={issues ? "warning" : "check_circle"} size={15} />
        {issues ? (lagging ? t("live.health.lagShort", { seconds }) : t("live.health.title")) : t("live.health.allGood")}
        <Icon name={open ? "expand_less" : "expand_more"} size={15} />
      </button>
      {open && (
        <ul id="live-health-rows" className="m-0 mt-1.5 grid list-none gap-1 rounded-row border border-line bg-surface p-1.5 sm:grid-cols-2">
          {rows.map((r) => (
            <li key={r.id} data-row={r.id} data-warn={r.warn ? "true" : undefined} className="flex items-start gap-2 rounded-seg px-2 py-1.5">
              <Icon name={r.icon} size={16} className={cn("mt-0.5", r.warn ? "text-warn" : "text-muted")} />
              <div className="text-small min-w-0">
                <b className="block font-semibold">{r.title}</b>
                <span className={r.warn ? "text-warn" : "text-muted"}>{r.detail}</span>
                {r.hint && <span className="block text-muted">{r.hint}</span>}
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
