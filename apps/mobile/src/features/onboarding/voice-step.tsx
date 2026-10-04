// SPDX-License-Identifier: Apache-2.0
// M1 step 3 (optional): a 20-second voice profile so the user shows up as
// "Me". The consent checkbox comes first; the passage and the microphone only
// after it. Anything but a saved profile leaves no consent and no audio behind.
import { Banner, cn, Icon, PhoneButton } from "@ghi/ui";
import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ipc } from "../../ipc";
import { EnrollError, EnrollMeter } from "../voice/enroll-panel";
import { useVoiceEnroll } from "../voice/use-voice-enroll";
import { StepLayout } from "./step-layout";

export function VoiceStep({ onNext }: { onNext: () => void }) {
  const { t } = useTranslation();
  const checkbox = useId();
  const [consent, setConsent] = useState(false);
  const [consentFailed, setConsentFailed] = useState(false);
  const [modelReady, setModelReady] = useState(true);
  const enroll = useVoiceEnroll();
  const { state } = enroll;
  const phase = state.phase === "done" ? "saved" : state.phase === "idle" ? "idle" : "listening";
  const last = useRef({ consent, saved: phase === "saved" });
  useEffect(() => {
    last.current = { consent, saved: phase === "saved" };
  });

  useEffect(() => {
    let alive = true;
    void ipc.commands
      .voiceStatus()
      .then(
        (r) => alive && r.status === "ok" && setModelReady(r.data.modelReady),
      );
    return () => {
      alive = false;
      // Leaving before a profile was saved takes the consent back (the hook
      // already ends the enrollment and wipes the audio).
      const { consent: given, saved } = last.current;
      if (given && !saved) void ipc.commands.voiceSetConsent(false);
    };
  }, []);

  const toggle = async (given: boolean) => {
    setConsent(given);
    setConsentFailed(false);
    const r = await ipc.commands.voiceSetConsent(given);
    if (r.status === "error") {
      setConsent(!given);
      setConsentFailed(true);
    }
  };

  return (
    <StepLayout
      icon="record_voice_over"
      title={t("mobile.voice.title")}
      subtitle={t("mobile.voice.body")}
      footer={
        phase === "saved" ? (
          <PhoneButton onClick={onNext}>
            {t("mobile.common.continue")}
          </PhoneButton>
        ) : (
          <PhoneButton variant="secondary" disabled={state.phase === "starting" || state.phase === "saving"} onClick={onNext}>
            {t("mobile.common.skip")}
          </PhoneButton>
        )
      }
    >
      <div className="flex flex-col gap-4">
        {!modelReady && (
          <Banner variant="info" title={t("mobile.voice.needsModels")} />
        )}
        {state.error && <EnrollError code={state.error} skippable />}
        {consentFailed && <EnrollError code="consent" skippable />}

        <label
          htmlFor={checkbox}
          className="text-ios-subhead flex min-h-ios-target cursor-pointer items-start gap-3 rounded-(--ios-radius-group) bg-surface px-4 py-3"
        >
          <input
            id={checkbox}
            type="checkbox"
            checked={consent}
            disabled={phase !== "idle" || !modelReady}
            onChange={(e) => void toggle(e.target.checked)}
            className="mt-0.5 size-5 shrink-0 accent-(--accent)"
          />
          <span>{t("mobile.voice.consent")}</span>
        </label>

        {consent && (
          <figure
            className={cn(
              "m-0 rounded-(--ios-radius-group) border border-line bg-surface px-4 py-3",
              phase === "listening" && "border-accent",
            )}
          >
            <figcaption className="text-ios-footnote font-medium text-muted">
              {t("mobile.voice.passageLabel")}
            </figcaption>
            <blockquote className="text-ios-body m-0 mt-1 font-serif">
              {t("mobile.voice.passage")}
            </blockquote>
          </figure>
        )}

        {consent && phase === "idle" && (
          <PhoneButton icon="mic" onClick={() => void enroll.start()}>
            {t("mobile.voice.start")}
          </PhoneButton>
        )}
        {phase === "listening" && (
          <>
            <p role="status" className="text-ios-subhead m-0 flex items-center gap-2 text-accent">
              <Icon name="graphic_eq" size={22} className="size-[1.375rem]" />
              {t("mobile.voice.listening")}
            </p>
            <EnrollMeter state={state} />
            <PhoneButton disabled={!enroll.canFinish} onClick={() => void enroll.finish()}>
              {t("mobile.voice.done")}
            </PhoneButton>
            <PhoneButton variant="ghost" onClick={enroll.cancel}>
              {t("mobile.common.cancel")}
            </PhoneButton>
          </>
        )}
        {phase === "saved" && (
          <p
            role="status"
            className="text-ios-body m-0 flex items-center gap-2 font-semibold text-accent"
          >
            <Icon name="check_circle" size={24} className="size-6" />
            {t("mobile.voice.saved")}
          </p>
        )}
      </div>
    </StepLayout>
  );
}
