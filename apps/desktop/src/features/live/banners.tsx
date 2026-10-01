// SPDX-License-Identifier: Apache-2.0
// System states as inline banners above the transcript (never modals while
// recording): record-only, asleep, silent system track, lost devices, disk,
// paused, waiting for audio.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { Icon, cn, usePlatform, type IconName } from "@ghi/ui";
import { useLive } from "../../state/live";
import { NO_AUDIO_MS, formatBytes, minutesLeft, waitingForAudio } from "./logic";
import type { ReactNode } from "react";

type Tone = "warn" | "rec" | "info";
const TONE: Record<Tone, string> = { warn: "bg-warn-soft text-warn", rec: "bg-rec-soft text-rec-ink", info: "bg-surface2 text-ink" };

function Banner({ id, tone = "warn", icon, children }: { id: string; tone?: Tone; icon: IconName; children: ReactNode }) {
  return (
    <p role={tone === "rec" ? "alert" : "status"} data-banner={id} className={cn("text-body m-0 flex items-start gap-2 rounded-row px-3.5 py-2.5", TONE[tone])}>
      <Icon name={icon} size={18} className="mt-px flex-none" />
      <span className="min-w-0">{children}</span>
    </p>
  );
}

/**
 * True after 10 s without a level while recording; any level, a pause or a
 * wake restarts the wait. Levels arrive 10x a second, so they only touch a
 * local variable; state changes at most once a second.
 */
function useNoAudio(): boolean {
  const [waiting, setWaiting] = useState(false);
  useEffect(() => {
    let lastLevelAtMs = Date.now();
    const unsub = useLive.subscribe((s, prev) => {
      const flows = s.levels !== prev.levels && (s.levels.mic != null || s.levels.system != null);
      if (flows || s.state !== prev.state || s.capture.asleep !== prev.capture.asleep) {
        lastLevelAtMs = Date.now();
        setWaiting(false);
      }
    });
    const id = window.setInterval(() => {
      const s = useLive.getState();
      setWaiting(waitingForAudio({ recording: s.state === "recording", asleep: s.capture.asleep, lastLevelAtMs, nowMs: Date.now() }));
    }, 1000);
    return () => {
      unsub();
      window.clearInterval(id);
    };
  }, []);
  return waiting;
}

export function LiveBanners() {
  const { t, i18n } = useTranslation();
  const platform = usePlatform();
  const s = useLive(useShallow((x) => ({ state: x.state, recordOnly: x.recordOnly, ...x.capture })));
  const noAudio = useNoAudio();
  const lost = (track: number) => s.lostTracks.includes(track);
  return (
    <div data-testid="live-banners" className="flex flex-col gap-2 empty:hidden">
      {s.recordOnly && (
        <Banner id="record-only" icon="info">
          {t("live.deferredBanner")}
        </Banner>
      )}
      {s.state === "paused" && (
        <Banner id="paused" tone="info" icon="pause_circle">
          <b>{t("live.paused.title")}</b> <span className="text-muted">{t("live.paused.subtitle")}</span>
        </Banner>
      )}
      {s.asleep && (
        <Banner id="asleep" icon="schedule">
          {t("live.banner.asleep")}
        </Banner>
      )}
      {s.systemSilent && (
        <Banner id="system-silent" icon="volume_off">
          {t("system.audioAccessOff")} {t("live.banner.roomHint")}
        </Banner>
      )}
      {lost(0) && (
        <Banner id="mic-lost" icon="mic_off">
          {t("live.banner.micLost")}
        </Banner>
      )}
      {lost(1) && (
        <Banner id="system-lost" icon="volume_off">
          {t("live.banner.systemLost")}
        </Banner>
      )}
      {s.diskFull ? (
        <Banner id="disk-full" tone="rec" icon="hard_drive">
          {t("live.banner.diskFull")}
        </Banner>
      ) : (
        s.diskLowBytes != null && (
          <Banner id="disk-low" icon="hard_drive">
            {t("system.diskLow", { free: formatBytes(s.diskLowBytes, i18n.language), minutes: minutesLeft(s.diskLowBytes) })}
          </Banner>
        )
      )}
      {noAudio && (
        <Banner id="no-audio" icon="volume_off">
          {t("live.noAudio", { context: platform, meetingApp: t("live.meetingAppFallback"), seconds: NO_AUDIO_MS / 1000 })}
        </Banner>
      )}
    </div>
  );
}
