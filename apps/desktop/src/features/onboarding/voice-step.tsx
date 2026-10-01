// SPDX-License-Identifier: Apache-2.0
// D1 step 5, "Your voice": built but skipped by the flow while
// `settings.voiceProfilesMe` is false (speaker embeddings arrive in phase 14).
// Consent comes first: nothing is read or stored until the box is checked.
//
// TODO(phase 14): enrolling is a timed stand-in for now; it must call the
// voice-profile command, which stores only after this consent.
import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Icon, usePlatform } from "@ghi/ui";
import { StepActions, StepFrame, type StepNav } from "./step-frame";

const READ_SECONDS = 20;

export function VoiceStep({ nav, readSeconds = READ_SECONDS }: { nav: StepNav; readSeconds?: number }) {
  const { t } = useTranslation();
  const context = usePlatform();
  const consentId = useId();
  const [consent, setConsent] = useState(false);
  const [phase, setPhase] = useState<"idle" | "listening" | "done">("idle");
  const [progress, setProgress] = useState(0);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearInterval(timer.current), []);

  const start = () => {
    const t0 = Date.now();
    setPhase("listening");
    timer.current = window.setInterval(() => {
      const p = Math.min(100, ((Date.now() - t0) / (readSeconds * 1000)) * 100);
      setProgress(p);
      if (p >= 100) {
        window.clearInterval(timer.current);
        setPhase("done");
      }
    }, 100);
  };

  return (
    <StepFrame title={t("onboarding.voice.title")} body={t("onboarding.voice.body")}>
      <p lang="vi" className="m-0 rounded-xl bg-surface2 px-5 py-4 font-serif text-[19px] leading-[1.7]">
        {t("onboarding.voice.passage")}
      </p>
      <div role="progressbar" aria-label={t("onboarding.voice.title")} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(progress)} className="h-2 overflow-hidden rounded bg-sunk">
        <i className="block h-full bg-accent" style={{ width: `${progress}%` }} />
      </div>
      <div className="flex items-start gap-2.5 text-[13px] leading-normal">
        <input id={consentId} type="checkbox" checked={consent} onChange={(e) => setConsent(e.target.checked)} disabled={phase !== "idle"} className="mt-0.5 size-4 accent-[var(--accent)]" />
        <label htmlFor={consentId}>{t(`onboarding.voice.consent_${context}`)}</label>
      </div>
      <div role="status" className="flex min-h-6 items-center gap-1.5 text-[14px] font-semibold text-accent">
        {phase === "listening" && (
          <>
            <Icon name="graphic_eq" size={19} />
            {t("onboarding.voice.listening")}
          </>
        )}
        {phase === "done" && (
          <>
            <Icon name="check_circle" size={19} />
            {t("onboarding.voice.done")}
          </>
        )}
      </div>
      <StepActions
        nav={nav}
        start={
          phase === "idle" && (
            <Button size="lg" variant="primary" icon="mic" disabled={!consent} onClick={start}>
              {t("onboarding.voice.start")}
            </Button>
          )
        }
        primary={phase === "done" ? t("common.continue") : t("common.skip")}
        primaryProps={{ variant: phase === "done" ? "primary" : "ghost", className: phase === "done" ? undefined : "text-muted" }}
        enter={phase === "done"}
      />
    </StepFrame>
  );
}
