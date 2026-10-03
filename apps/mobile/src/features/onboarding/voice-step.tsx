// SPDX-License-Identifier: Apache-2.0
// M1 step 3 (optional): a 20-second voice profile so the user shows up as
// "Me". The consent checkbox comes first; the passage and the microphone only
// after it. Anything but a saved profile leaves no consent and no audio behind.
import { Banner, cn, Icon, PhoneButton } from "@ghi/ui";
import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ipc } from "../../ipc";
import { StepLayout } from "./step-layout";

type Phase = "idle" | "listening" | "saved";

export function VoiceStep({ onNext }: { onNext: () => void }) {
  const { t } = useTranslation();
  const checkbox = useId();
  const [consent, setConsent] = useState(false);
  const [phase, setPhase] = useState<Phase>("idle");
  const [failed, setFailed] = useState(false);
  const [modelReady, setModelReady] = useState(true);
  const state = useRef({ consent, phase });
  useEffect(() => {
    state.current = { consent, phase };
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
      // Leaving before a profile was saved takes the consent and the audio back.
      const { consent: given, phase: last } = state.current;
      if (last === "listening") void ipc.commands.voiceEnrollCancel();
      if (given && last !== "saved") void ipc.commands.voiceSetConsent(false);
    };
  }, []);

  const toggle = async (given: boolean) => {
    setConsent(given);
    setFailed(false);
    const r = await ipc.commands.voiceSetConsent(given);
    if (r.status === "error") setConsent(!given);
  };

  const start = async () => {
    setFailed(false);
    const r = await ipc.commands.voiceEnrollStart();
    if (r.status === "ok") setPhase("listening");
    else setFailed(true);
  };

  const stop = async () => {
    const r = await ipc.commands.voiceEnrollStop();
    if (r.status === "ok") setPhase("saved");
    else {
      setPhase("idle");
      setFailed(true);
    }
  };

  const cancel = async () => {
    await ipc.commands.voiceEnrollCancel();
    setPhase("idle");
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
          <PhoneButton variant="secondary" onClick={onNext}>
            {t("mobile.common.skip")}
          </PhoneButton>
        )
      }
    >
      <div className="flex flex-col gap-4">
        {!modelReady && (
          <Banner variant="info" title={t("mobile.voice.needsModels")} />
        )}
        {failed && <Banner variant="warning" title={t("mobile.voice.error")} />}

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
          <PhoneButton icon="mic" onClick={() => void start()}>
            {t("mobile.voice.start")}
          </PhoneButton>
        )}
        {phase === "listening" && (
          <>
            <p
              role="status"
              className="text-ios-subhead m-0 flex items-center gap-2 text-accent"
            >
              <Icon name="graphic_eq" size={22} className="size-[1.375rem]" />
              {t("mobile.voice.listening")}
            </p>
            <PhoneButton onClick={() => void stop()}>
              {t("mobile.voice.done")}
            </PhoneButton>
            <PhoneButton variant="ghost" onClick={() => void cancel()}>
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
