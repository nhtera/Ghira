// SPDX-License-Identifier: Apache-2.0
// System states as inline banners above the transcript (never modals while
// recording): record-only, asleep, silent system track, lost devices, disk,
// paused, waiting for audio.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { Icon, cn, useToast, usePlatform, type IconName } from "@ghi/ui";
import { useNavigate } from "@tanstack/react-router";
import { Button } from "@ghi/ui";
import { ipc } from "../../ipc";
import { BANNER_ACTION, SystemBanner } from "../system-states/system-banner";
import { useLive } from "../../state/live";
import { NO_AUDIO_MS, formatBytes, minutesLeft, splitOthers, waitingForAudio } from "./logic";
import type { ReactNode } from "react";

type Tone = "warn" | "rec" | "info";
const TONE: Record<Tone, string> = { warn: "bg-warn-soft text-warn", rec: "bg-rec-soft text-rec-ink", info: "bg-surface2 text-ink" };

function Banner({ id, tone = "warn", icon, action, children }: { id: string; tone?: Tone; icon: IconName; action?: ReactNode; children: ReactNode }) {
  const info = tone === "info";
  return (
    <div role={tone === "rec" ? "alert" : "status"} data-banner={id} className={cn("flex flex-none items-center gap-2.5 rounded-ctl text-[13px]", info ? "px-3.5 py-3 font-normal" : "px-3 py-2.5 font-medium", TONE[tone])}>
      <Icon name={icon} size={info ? 19 : 18} className="flex-none" />
      <span className="min-w-0 flex-1 leading-normal">{children}</span>
      {action}
    </div>
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

/** How long the "more than eight voices" note stays (it is news once, not a standing warning). */
export const MANY_VOICES_MS = 20_000;

/**
 * True for a while after the first speaker of this meeting lands in Others,
 * and not again for the same meeting (a later speaker doesn't bring it back).
 */
function useManyVoices(): boolean {
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    let firedFor: string | null = null;
    let seen: string | null = null;
    let timer: number | undefined;
    const check = (s: ReturnType<typeof useLive.getState>) => {
      if (s.meeting !== seen) {
        // Another meeting: the old note goes, and this one may have its own.
        seen = s.meeting;
        window.clearTimeout(timer);
        setVisible(false);
      }
      if (s.meeting && firedFor !== s.meeting && splitOthers(Object.values(s.speakers)).others.length > 0) {
        firedFor = s.meeting;
        setVisible(true);
        window.clearTimeout(timer);
        timer = window.setTimeout(() => setVisible(false), MANY_VOICES_MS);
      }
    };
    const unsub = useLive.subscribe(check);
    // Already in the store when this mounts (a window opened mid-meeting).
    const first = window.setTimeout(() => check(useLive.getState()), 0);
    return () => {
      unsub();
      window.clearTimeout(first);
      window.clearTimeout(timer);
    };
  }, []);
  return visible;
}

/** Inline notes above the transcript: waiting for models, no sound, many voices. */
export function LiveBanners() {
  const { t } = useTranslation();
  const platform = usePlatform();
  const s = useLive(useShallow((x) => ({ state: x.state, recordOnly: x.recordOnly, ...x.capture })));
  const { show } = useToast();
  const noAudio = useNoAudio();
  const checkSource = async () => {
    // Room mode has no system track; a missing mic level points at the mic.
    const { session, levels } = useLive.getState();
    const pane = session?.mode === "room" || levels.mic == null ? "microphone" : "systemAudio";
    const r = await ipc.commands.openPrivacySettings(pane);
    if (r.status === "error") show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
  };
  const manyVoices = useManyVoices();
  return (
    <div data-testid="live-banners" className="flex flex-none flex-col gap-2 empty:hidden">
      {s.recordOnly && (
        <Banner id="record-only" tone="info" icon="hourglass_top">
          {t("live.deferredBanner")}
        </Banner>
      )}
      {manyVoices && (
        <Banner id="many-voices" tone="info" icon="groups">
          {t("live.manyVoices")}
        </Banner>
      )}
      {noAudio && (
        <Banner
          id="no-audio"
          icon="volume_off"
          action={
            <button type="button" onClick={() => void checkSource()} className="h-7 flex-none rounded-seg border border-current px-2.5 text-[12px] font-semibold">
              {t("live.checkSource")}
            </button>
          }
        >
          {t("live.noAudio", { context: platform, meetingApp: t("live.meetingAppFallback"), seconds: NO_AUDIO_MS / 1000 })}
        </Banner>
      )}
    </div>
  );
}

/** True after the core fell back to all system audio (no meeting app found) for the current meeting, until dismissed. */
function useAppAudioFallback() {
  const [meeting, setMeeting] = useState<string | null>(null);
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void ipc
      .onCoreEvent((env) => {
        if (env.event.type === "appAudioFallback") setMeeting(env.event.meeting);
      })
      .then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  }, []);
  return { shown: meeting != null, dismiss: () => setMeeting(null) };
}

/** Capture conditions as full-width strips under the title bar. */
export function LiveSystemBanners() {
  const { t, i18n } = useTranslation();
  const platform = usePlatform();
  const navigate = useNavigate();
  const { show } = useToast();
  const s = useLive(useShallow((x) => ({ state: x.state, ...x.capture })));
  // Dismissed at this much free space: it comes back if the space keeps falling (by another quarter).
  const [dismissedAt, setDismissedAt] = useState<number | null>(null);
  const diskDismissed = dismissedAt != null && s.diskLowBytes != null && s.diskLowBytes >= dismissedAt * 0.75;
  const fallback = useAppAudioFallback();
  const lost = (track: number) => s.lostTracks.includes(track);
  const openAudioSettings = async () => {
    const r = await ipc.commands.openPrivacySettings("systemAudio");
    if (r.status === "error") show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
  };
  const manage = (
    <Button size="sm" variant="ghost" className={BANNER_ACTION} onClick={() => void navigate({ to: "/settings/$section", params: { section: "privacy" } })}>
      {t("system.manageStorage")}
    </Button>
  );
  return (
    <div data-testid="live-system-banners" className="flex flex-none flex-col empty:hidden">
      {s.asleep && (
        <SystemBanner id="asleep" icon="schedule">
          {t("live.banner.asleep")}
        </SystemBanner>
      )}
      {fallback.shown && (
        <SystemBanner id="app-audio-fallback" tone="info" icon="info" onDismiss={fallback.dismiss}>
          {t("live.banner.appAudioFallback")}
        </SystemBanner>
      )}
      {s.systemSilent && (
        <SystemBanner
          id="system-silent"
          icon="volume_off"
          actions={
            <Button size="sm" variant="ghost" className={BANNER_ACTION} onClick={() => void openAudioSettings()}>
              {t(`common.openSystemSettings_${platform}`)}
            </Button>
          }
        >
          {t("system.audioAccessOff")} {t("live.banner.roomHint")}
        </SystemBanner>
      )}
      {lost(0) && (
        <SystemBanner id="mic-lost" icon="mic_off">
          {t("live.banner.micLost")}
        </SystemBanner>
      )}
      {lost(1) && (
        <SystemBanner
          id="system-lost"
          icon="volume_off"
          actions={
            <Button size="sm" variant="ghost" className={BANNER_ACTION} onClick={() => void openAudioSettings()}>
              {t(`common.openSystemSettings_${platform}`)}
            </Button>
          }
        >
          {t("live.banner.systemLost")}
        </SystemBanner>
      )}
      {s.diskFull ? (
        <SystemBanner id="disk-full" tone="rec" icon="hard_drive" actions={manage}>
          {t("live.banner.diskFull")}
        </SystemBanner>
      ) : (
        s.diskLowBytes != null &&
        !diskDismissed && (
          <SystemBanner id="disk-low" icon="hard_drive" actions={manage} onDismiss={() => setDismissedAt(s.diskLowBytes)}>
            {t("system.diskLow", { free: formatBytes(s.diskLowBytes, i18n.language), minutes: minutesLeft(s.diskLowBytes) })}
          </SystemBanner>
        )
      )}
    </div>
  );
}
