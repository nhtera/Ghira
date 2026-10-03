// SPDX-License-Identifier: Apache-2.0
// Health indicator (brief D4): one quiet line that opens into rows (speech
// recognition, speaker detection, audio route, disk). Warnings also come up
// as inline banners; this is where the numbers live.
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { useQueryClient } from "@tanstack/react-query";
import { Button, Icon, Popover, cn, useToast, usePlatform, type IconName } from "@ghi/ui";
import { ipc } from "../../ipc";
import { settingsQuery } from "../../shell/root-view";
import { useLive } from "../../state/live";
import { LAG_WARN_S, formatBytes, minutesLeft } from "./logic";

type Row = { id: string; icon: IconName; title: string; detail: string; warn: boolean; hint?: ReactNode };

export function Health() {
  const { t, i18n } = useTranslation();
  const platform = usePlatform();
  const [open, setOpen] = useState(false);
  const { show } = useToast();
  const queryClient = useQueryClient();
  const h = useLive(useShallow((s) => ({ lag: s.asrLagS, aec: s.aec, hfp: s.capture.bluetoothHfp, low: s.capture.diskLowBytes, full: s.capture.diskFull, recordOnly: s.recordOnly })));
  const seconds = Math.round(h.lag * 10) / 10;
  const lagging = h.lag > LAG_WARN_S;
  // The lag hint acts: Fast mode applies from the next recording.
  const switchToFast = async () => {
    const r = await ipc.commands.updateSettings({ liveMode: "fast" });
    if (r.status === "ok") {
      queryClient.setQueryData(settingsQuery.queryKey, r.data);
      show({ tone: "success", title: t("live.health.fastOn") });
    } else show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
  };
  const rows: Row[] = [
    { id: "asr", icon: "subtitles", title: t("live.health.transcription.title"), detail: h.recordOnly ? t("live.health.recordOnly") : t("live.health.transcription.detail", { context: platform, seconds }), warn: lagging },
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
      <Popover
        open={open}
        onOpenChange={setOpen}
        side="top"
        align="start"
        label={t("live.health.title")}
        className="flex w-[360px] flex-col rounded-panel p-2"
        trigger={
          <button
            type="button"
            className={cn("inline-flex h-7 items-center gap-1.5 rounded-full border border-line pr-2 pl-2 text-[12px] font-semibold", issues ? "bg-warn-soft text-warn" : "bg-surface text-muted hover:bg-sunk")}
          >
            <Icon name={issues ? "warning" : "check_circle"} size={15} />
            {issues ? (lagging ? t("live.health.lagShort", { seconds }) : t("live.health.title")) : t("live.health.allGood")}
            <Icon name={open ? "expand_more" : "expand_less"} size={15} />
          </button>
        }
      >
        <ul className="m-0 flex list-none flex-col p-0">
          {rows.map((r) => (
            <li key={r.id} data-row={r.id} data-warn={r.warn ? "true" : undefined} className="flex items-center gap-2.5 p-2">
              <Icon name={r.icon} size={18} className="flex-none text-muted" />
              <div className="min-w-0 flex-1 text-[13px]">
                <b className="block font-medium">{r.title}</b>
                {r.hint && <span className="block text-[12px] text-muted">{r.hint}</span>}
              </div>
              <span className={cn("max-w-[55%] text-right text-[12px]", r.warn ? "text-warn" : "text-muted")}>{r.detail}</span>
            </li>
          ))}
        </ul>
        {lagging && (
          <Button variant="primary" onClick={() => void switchToFast()} className="mx-2 mt-1 mb-1.5">
            {t("live.health.switchToFast")}
          </Button>
        )}
      </Popover>
    </div>
  );
}
