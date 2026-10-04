// SPDX-License-Identifier: Apache-2.0
// The live screen's header (design D4): editable title, the session's mode,
// language and audio route, the Transcript/Focus layout, the clock with Pause,
// the mini recorder and Stop. The clock's one-second tick lives here.
import { formatClock } from "@ghi/i18n";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { Button, Icon, Menu, Segmented, cn, type IconName } from "@ghi/ui";
import type { RecordMode } from "../../bindings";
import { ipc } from "../../ipc";
import { useAppActions } from "../../shell/actions";
import { elapsedMs, useLive } from "../../state/live";
import { useNow } from "./clock";
import { HeaderRecord } from "./header-record";
import { TitleField, useConsent } from "./toolbar";
import { SensitiveBadge } from "../sensitive";

export type LiveLayout = "transcript" | "focus";

const MODES: Array<{ value: RecordMode; icon: IconName }> = [
  { value: "call", icon: "videocam" },
  { value: "room", icon: "groups" },
];

/** Call / Room as the segmented control looks, but read-only: a session keeps its mode. */
function ModeBadge({ mode, compact }: { mode: RecordMode; compact: boolean }) {
  const { t } = useTranslation();
  return (
    <div className="inline-flex flex-none gap-0.5 rounded-ctl border border-ctl p-0.5">
      {MODES.map((m) => (
        <span
          key={m.value}
          aria-current={m.value === mode ? "true" : undefined}
          title={compact ? t(`record.mode.${m.value}`) : undefined}
          className={cn("inline-flex h-6 items-center gap-1 rounded-seg px-2.5 text-[12px] font-medium", m.value === mode ? "bg-accent-soft text-accent" : "text-muted")}
        >
          <Icon name={m.icon} size={14} />
          {compact ? <span className="sr-only">{t(`record.mode.${m.value}`)}</span> : t(`record.mode.${m.value}`)}
        </span>
      ))}
    </div>
  );
}

function Clock() {
  const state = useLive((s) => s.state);
  const clock = useLive(useShallow((s) => ({ startedAtMs: s.startedAtMs, pausedAtMs: s.pausedAtMs, pausedTotalMs: s.pausedTotalMs })));
  const now = useNow(state === "recording");
  return (
    <span className="flex flex-none items-center gap-2">
      <span aria-hidden="true" className={cn("size-2.5 rounded-full", state === "recording" ? "bg-rec" : "bg-faint")} />
      <span data-testid="live-clock" className="min-w-[62px] text-mono text-[20px] font-medium tabular-nums">
        {formatClock(elapsedMs(clock, now), { pad: true })}
      </span>
    </span>
  );
}

export function LiveHeader({
  meeting,
  mode,
  layout,
  onLayout,
  onDiscard,
  onSensitive,
  discardSeconds,
  compact,
}: {
  meeting: string;
  mode: RecordMode;
  layout: LiveLayout;
  onLayout: (l: LiveLayout) => void;
  onDiscard: (seconds: number) => void;
  /** Asks to make the recording sensitive (the screen shows the confirmation). */
  onSensitive: () => void;
  discardSeconds: number[];
  compact: boolean;
}) {
  const { t } = useTranslation();
  const session = useLive((s) => s.session);
  const state = useLive((s) => s.state);
  const aec = useLive((s) => s.aec);
  const consent = useConsent(meeting);
  const { pauseRecording, resumeRecording, stopRecording } = useAppActions();
  const shownMode: RecordMode = session?.mode === "room" ? "room" : session?.mode === "call" ? "call" : mode;
  const language = session?.language === "en" ? t("onboarding.languages.english") : session?.language === "vi" ? t("onboarding.languages.vietnamese") : t("record.languages");
  const route = aec ? t("live.audio.speakersAecOn") : t("live.audio.headphonesAecOff");
  const running = state === "recording" || state === "paused";
  // Opening a native window can't fail in a way the user can act on.
  const openMini = () => void ipc.commands.openMiniRecorder();

  return (
    <header data-tauri-drag-region className="@container flex flex-none items-center gap-2.5 border-b border-line px-5 py-3">
      <TitleField key={`${meeting}-${session ? "s" : "n"}`} meeting={meeting} initial={session?.title ?? ""} />
      <ModeBadge mode={shownMode} compact={compact} />
      <span title={language} className="hidden h-[30px] flex-none items-center gap-1 px-1 text-[12px] whitespace-nowrap text-muted @[1140px]:flex">
        <Icon name="translate" size={16} />
        {language}
      </span>
      <span
        title={route}
        data-aec={aec ? "on" : "off"}
        className={cn("inline-flex h-7 flex-none items-center gap-1 rounded-full border border-line px-2.5 text-[12px] font-medium whitespace-nowrap", aec ? "bg-accent-soft text-accent" : "text-muted")}
      >
        <Icon name={aec ? "speaker" : "headphones"} size={16} />
        <span className="sr-only @[1140px]:not-sr-only">{aec ? t("live.audio.speakers") : t("live.audio.headphones")}</span>
        <span className="sr-only">{route}</span>
      </span>
      {session?.sensitive && <SensitiveBadge />}
      {consent.confirmed && (
        <span title={t("live.consent.confirmed")} data-testid="consent-confirmed" className="inline-flex h-7 flex-none items-center gap-1 text-[12px] font-medium whitespace-nowrap text-accent">
          <Icon name="check_circle" size={16} />
          <span className="sr-only @[1140px]:not-sr-only">{t("live.consent.confirmed")}</span>
        </span>
      )}
      {running && (
        <Segmented
          label={t("live.layout")}
          value={layout}
          onChange={onLayout}
          className="flex-none"
          options={[
            { value: "transcript", label: t("live.layoutTranscript"), icon: "subtitles" },
            { value: "focus", label: t("live.layoutFocus"), icon: "edit" },
          ]}
        />
      )}
      {running ? (
        <>
          <span aria-hidden="true" className="mx-1 h-6 w-px flex-none bg-line" />
          <Clock />
          <Button icon={state === "paused" ? "play_arrow" : "pause"} onClick={() => void (state === "paused" ? resumeRecording() : pauseRecording())}>
            {state === "paused" ? t("live.resume") : t("live.pause")}
          </Button>
          <Button icon="picture_in_picture_alt" onClick={openMini} aria-label={t("live.mini")} title={t("live.mini")} data-testid="mini-recorder" />
          <Button variant="danger" icon="stop" onClick={() => void stopRecording()}>
            {t("live.stop")}
          </Button>
          <Menu
            label={t("live.more")}
            trigger={<Button icon="more_horiz" aria-label={t("live.more")} />}
            items={[
              { kind: "checkbox", label: t("live.consent.confirmed"), checked: consent.confirmed, onSelect: () => void consent.toggle() },
              // One way: part of the audio is gone, so a recording stays sensitive.
              session?.sensitive
                ? { label: t("sensitive.menu"), icon: "check" as const, disabled: true, hint: t("sensitive.liveLocked"), onSelect: () => {} }
                : { label: t("sensitive.menuLive"), icon: "visibility_off" as const, movesFocus: true, onSelect: onSensitive },
              { kind: "separator" },
              ...discardSeconds.map((s) => ({ label: t("live.discard.menuItem", { count: s / 60 }), icon: "delete" as const, danger: true, onSelect: () => onDiscard(s) })),
            ]}
          />
        </>
      ) : (
        <HeaderRecord />
      )}
    </header>
  );
}
