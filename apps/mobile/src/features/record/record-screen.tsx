// SPDX-License-Identifier: Apache-2.0
// M2: the Record tab, laid out like the design: the title and the privacy pill,
// the big mono timer, then (while recording) the level waveform, the banners
// the phase and shell events ask for and the flat live transcript, and the
// round Mark / Stop / Pause controls pinned at the bottom. Idle: the processing
// choice above the record ring.
// Starting goes through the consent reminder (M2) or, while a phone call is
// active, the call notice (M6). The controls are pinned at the bottom; the
// transcript is what gives way at big text sizes.
import { cn, Icon, PrivacyIndicator, type PrivacyState } from "@ghi/ui";
import { formatClock } from "@ghi/i18n";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { RecordPhase } from "../../bindings";
import { ipc } from "../../ipc";
import { CalendarCard, useCurrentEvent } from "../calendar";
import { CallNoticeSheet } from "../consent/call-notice-sheet";
import { ConsentSheet } from "../consent/consent-sheet";
import { SensitiveBadge, SensitiveSheet } from "../sensitive";
import { DiscardSheet } from "./discard-sheet";
import { InterruptionSheet } from "./interruption-sheet";
import { LiveTranscript, TurnAnnouncer } from "./live-transcript";
import { MoreSheet } from "./more-sheet";
import { hasSession, isCapturing, type RecordModel } from "./model";
import { PhaseBanners } from "./phase-banners";
import { RecordControls } from "./record-controls";
import { TargetPicker } from "./target-picker";
import { bannerError, useRecord, useRecordSetup } from "./use-record";
import { Waveform } from "./waveform";

type ControlState = "idle" | "starting" | "recording" | "paused" | "stopping" | "error";

export function controlState(model: Pick<RecordModel, "phase" | "meeting">, starting: boolean, micDenied: boolean): ControlState {
  const { phase } = model;
  if (isCapturing(model)) return "recording";
  if (starting || phase === "loading") return "starting";
  if (phase === "finishing") return "stopping";
  if (phase === "paused" || phase === "interrupted") return "paused";
  return micDenied ? "error" : "idle";
}

function privacyState(model: RecordModel): PrivacyState {
  if (isCapturing(model)) return "recording";
  const phase: RecordPhase = model.phase;
  return phase === "paused" || phase === "interrupted" ? "paused" : "local";
}

export function RecordScreen() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { setup, patch } = useRecordSetup();
  const rec = useRecord(setup);
  const { model } = rec;
  const [sheet, setSheet] = useState<"consent" | "call" | null>(null);
  // Chosen in the start sheet, for that one recording.
  const [sensitiveNext, setSensitiveNext] = useState(false);
  const [more, setMore] = useState<"menu" | "sensitive" | null>(null);
  const [discard, setDiscard] = useState<number | null>(null);
  const active = hasSession(model);
  const capturing = isCapturing(model);
  // The event the next recording will be named after (idle only: a recording already has its name).
  const calendarEvent = useCurrentEvent(!active);

  const confirm = async () => {
    const call = sheet === "call";
    setSheet(null);
    const refused = await rec.start({ consent: true, call, sensitive: sensitiveNext });
    if (refused === null) setSensitiveNext(false);
    // A call began after the screen last looked: show the call notice instead.
    if (refused === "callActive") {
      patch({ callActive: true });
      setSheet("call");
    } else if (refused === "microphoneDenied") patch({ mic: "denied" });
  };

  const sensitiveOption = setup.recordOnlyDevice ? undefined : { checked: sensitiveNext, onChange: setSensitiveNext };
  const marksLabel = model.marks > 0 ? `${t("mobile.record.mark")}, ${t("mobile.record.marks", { count: model.marks })}` : t("mobile.record.mark");

  const clock = formatClock(model.elapsedS * 1000, { pad: true });
  const paused = model.phase === "paused" || model.phase === "interrupted";
  const controls = (
    <RecordControls
      state={controlState(model, rec.starting, setup.mic === "denied")}
      capturing={capturing}
      paused={paused}
      marks={model.marks}
      markLabel={marksLabel}
      onMark={rec.mark}
      onStart={() => {
        setSensitiveNext(false);
        setSheet(setup.callActive ? "call" : "consent");
      }}
      onPause={rec.pause}
      onResume={rec.resume}
      onStop={rec.stop}
      onFix={() => void ipc.commands.openAppSettings()}
    />
  );

  return (
    <section data-screen="record" className="flex h-full flex-col gap-3 overflow-hidden bg-surface px-5 pt-[calc(var(--safe-top)+12px)] pb-4">
      <header className="flex shrink-0 flex-wrap items-center justify-between gap-x-3 gap-y-1">
        <h1 className={cn("m-0", active ? "text-ios-title2" : "text-ios-headline font-bold")}>{t(active ? "mobile.record.title" : "mobile.record.titleIdle")}</h1>
        {/* The design's idle frame has no pill: the note under the ring says the audio stays here. */}
        {active && <PrivacyIndicator state={privacyState(model)} />}
      </header>

      <div className={cn("relative shrink-0", !active && "pt-7")}>
        <p
          role="timer"
          data-testid="timer"
          // Fits between the margins and the More button: mono digits are 0.6 em wide, so an hour+ time (1:02:33) shrinks instead of sliding under it.
          style={{ fontSize: `min(3rem, calc((100vw - 9rem) / ${clock.length * 0.6}))` }}
          className={cn("m-0 text-center font-mono leading-none font-medium tracking-[-0.02em] tabular-nums", capturing ? "text-ink" : "text-faint")}
        >
          <span className="sr-only">{t("mobile.record.elapsed")} </span>
          {clock}
        </p>
        {active && (
          <button
            type="button"
            aria-label={t("mobile.record.more")}
            disabled={!capturing && model.phase !== "paused"}
            onClick={() => setMore("menu")}
            className="absolute end-0 top-1/2 grid size-ios-target -translate-y-1/2 place-items-center rounded-full border border-ctl bg-surface2 text-ink active:bg-sunk disabled:cursor-not-allowed disabled:opacity-50"
          >
            <Icon name="more_horiz" size={22} className="size-[1.375rem]" />
          </button>
        )}
      </div>

      {/* Nothing to draw before a session: the room goes to the controls at big text sizes. */}
      {active && <Waveform active={capturing} tone={model.pocket ? "muffled" : "ok"} />}

      <PhaseBanners
        model={model}
        recordOnlyDevice={setup.recordOnlyDevice}
        saved={model.saved ? { onOpen: () => void navigate({ to: "/meetings" }), onDismiss: rec.dismissSaved } : undefined}
        error={bannerError(rec.error) ? { kind: bannerError(rec.error)!, onDismiss: rec.dismissError } : undefined}
        onDownloadModels={() => void navigate({ to: "/settings" })}
      />

      {active && model.sensitive && <SensitiveBadge />}
      <TurnAnnouncer announce={model.announce} />

      {active ? (
        <>
          <LiveTranscript lines={model.lines} partial={model.partial} speakers={model.speakers} className="min-h-0 flex-1" />
          {/* Pinned: the transcript above is what shrinks, at any text size. */}
          <div className="flex shrink-0 flex-col gap-3">{controls}</div>
        </>
      ) : (
        // The design puts the ring right under the clock; what follows scrolls at big text sizes.
        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pt-[18px]">
          <div className="flex shrink-0 justify-center">{controls}</div>
          <p className="text-ios-callout m-0 mt-3.5 flex shrink-0 items-center gap-2.5 rounded-[14px] bg-accent-soft p-3.5 font-semibold text-accent">
            <Icon name="speaker_phone" size={24} className="size-6 shrink-0" />
            {t("mobile.privacy.audioStaysLine")}
          </p>
          {calendarEvent && <CalendarCard event={calendarEvent} />}
          {!setup.recordOnlyDevice && <TargetPicker value={setup.target} onChange={(target) => patch({ target })} disabled={["desktop", "cloud"]} className="shrink-0" />}
        </div>
      )}

      {/* Sensitive mode keeps only the live transcript: a record-only phone has none to keep. */}
      <ConsentSheet language={setup.language} open={sheet === "consent"} onCancel={() => setSheet(null)} onConfirm={() => void confirm()} sensitive={sensitiveOption} />
      <CallNoticeSheet language={setup.language} open={sheet === "call"} onCancel={() => setSheet(null)} onConfirm={() => void confirm()} sensitive={sensitiveOption} />
      <MoreSheet
        open={more === "menu"}
        onClose={() => setMore(null)}
        sensitive={model.sensitive}
        canSensitive={model.phase !== "recordOnly" && !setup.recordOnlyDevice}
        onSensitive={() => setMore("sensitive")}
        onDiscard={(s) => {
          setMore(null);
          setDiscard(s);
        }}
      />
      <SensitiveSheet
        open={more === "sensitive"}
        recording
        onCancel={() => setMore(null)}
        onConfirm={() => {
          setMore(null);
          void rec.makeSensitive();
        }}
      />
      <DiscardSheet seconds={discard} sensitive={model.sensitive} behind={model.backlogS > 0} preview={rec.discardPreview} onConfirm={rec.discardFrom} onClose={() => setDiscard(null)} />
      <InterruptionSheet
        open={model.phase === "interrupted"}
        call={model.interruption?.call ?? setup.callActive}
        recordedS={model.interruption?.recordedS ?? model.elapsedS}
        onResume={rec.resume}
        onStop={rec.stop}
      />
    </section>
  );
}
