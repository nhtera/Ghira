// SPDX-License-Identifier: Apache-2.0
// M2: the Record tab. Idle: the processing choice and the thumb-zone record
// button. Recording: timer, level waveform, the live "Speaker N" transcript,
// Mark / Pause / Stop, and the banners the phase and shell events ask for.
// Starting goes through the consent reminder (M2) or, while a phone call is
// active, the call notice (M6). The controls are pinned at the bottom; the
// transcript is what gives way at big text sizes.
import { Icon, PhoneButton, PrivacyIndicator, type PrivacyState, RecordControl } from "@ghi/ui";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { RecordPhase } from "../../bindings";
import { ipc } from "../../ipc";
import { CallNoticeSheet } from "../consent/call-notice-sheet";
import { ConsentSheet } from "../consent/consent-sheet";
import { SensitiveBadge, SensitiveSheet } from "../sensitive";
import { DiscardSheet } from "./discard-sheet";
import { InterruptionSheet } from "./interruption-sheet";
import { LiveTranscript, TurnAnnouncer } from "./live-transcript";
import { MoreSheet } from "./more-sheet";
import { hasSession, isCapturing, type RecordModel } from "./model";
import { PhaseBanners } from "./phase-banners";
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

  return (
    <section data-screen="record" className="flex h-full flex-col gap-3 overflow-hidden px-4 pt-[calc(var(--safe-top)+12px)] pb-4">
      <header className="flex shrink-0 flex-wrap items-center justify-between gap-x-3 gap-y-1">
        <h1 className="text-ios-title2 m-0">{t("mobile.record.title")}</h1>
        <PrivacyIndicator state={privacyState(model)} />
      </header>

      <PhaseBanners
        model={model}
        recordOnlyDevice={setup.recordOnlyDevice}
        saved={model.saved ? { onOpen: () => void navigate({ to: "/meetings" }), onDismiss: rec.dismissSaved } : undefined}
        error={bannerError(rec.error) ? { kind: bannerError(rec.error)!, onDismiss: rec.dismissError } : undefined}
        onDownloadModels={() => void navigate({ to: "/settings" })}
      />

      {active && model.sensitive && <SensitiveBadge />}

      {/* Nothing to draw before a session: the room goes to the controls at big text sizes. */}
      {active && <Waveform active={capturing} />}

      {active ? (
        <LiveTranscript lines={model.lines} partial={model.partial} speakers={model.speakers} className="min-h-0 flex-1" />
      ) : (
        <p className="text-ios-subhead m-0 flex min-h-0 flex-1 flex-col items-center justify-center gap-2 text-center text-muted">
          <Icon name="mic" size={32} className="size-8" />
          {t("mobile.privacy.audioStaysLine")}
        </p>
      )}
      <TurnAnnouncer announce={model.announce} />

      {/* Pinned: the transcript above is what shrinks, at any text size. */}
      <div className="flex shrink-0 flex-col gap-3">
        {!active && !setup.recordOnlyDevice && <TargetPicker value={setup.target} onChange={(target) => patch({ target })} disabled={["desktop", "cloud"]} />}

        {active && (
          <div className="flex items-center justify-center gap-2">
            <PhoneButton variant="secondary" icon="star_outline" aria-label={marksLabel} disabled={!capturing} onClick={rec.mark} inline>
              {t("mobile.record.mark")}
              {model.marks > 0 && <span className="font-normal text-muted">{t("mobile.record.marks", { count: model.marks })}</span>}
            </PhoneButton>
            <button
              type="button"
              aria-label={t("mobile.record.more")}
              disabled={!capturing && model.phase !== "paused"}
              onClick={() => setMore("menu")}
              className="grid min-h-ios-target min-w-ios-target shrink-0 place-items-center rounded-(--ios-radius-group) border border-ctl bg-surface text-ink active:bg-sunk disabled:cursor-not-allowed disabled:opacity-50"
            >
              <Icon name="more_horiz" size={22} className="size-[1.375rem]" />
            </button>
          </div>
        )}

        <RecordControl
          state={controlState(model, rec.starting, setup.mic === "denied")}
          mode="room"
          elapsedMs={model.elapsedS * 1000}
          onStart={() => {
            setSensitiveNext(false);
            setSheet(setup.callActive ? "call" : "consent");
          }}
          onPause={rec.pause}
          onResume={rec.resume}
          onStop={rec.stop}
          onFix={() => void ipc.commands.openAppSettings()}
        />
      </div>

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
