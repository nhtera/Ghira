// SPDX-License-Identifier: Apache-2.0
// D1 step 5, "Your voice": real enrollment. Consent comes first: the mic is
// not opened and nothing is stored until the box is checked. Skip is always
// there, leaving mid-read releases the mic, and a voice model that is still
// downloading never blocks the step (the user skips and records later). The
// flow shows this step only while that model is installed or on its way.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@ghi/ui";
import { EnrollPanel, useConsentKey } from "../people/enroll-panel";
import { useInvalidatePeople, useVoiceStatus } from "../people/queries";
import { useVoiceEnroll } from "../people/use-voice-enroll";
import { StepActions, StepFrame, type StepNav } from "./step-frame";

export function VoiceStep({ nav, voiceDownloading = false, strictOffline = false }: { nav: StepNav; voiceDownloading?: boolean; strictOffline?: boolean }) {
  const { t } = useTranslation();
  const key = useConsentKey();
  const invalidate = useInvalidatePeople();
  const voice = useVoiceStatus({ pollWhileNotReady: true });
  const enroll = useVoiceEnroll(key, invalidate);
  const [consent, setConsent] = useState(false);
  const { phase, error } = enroll.state;
  // Until the status is known, assume ready: a failing start says why.
  const modelReady = voice.data?.modelReady ?? true;

  return (
    <StepFrame title={t("onboarding.voice.title")} body={t("onboarding.voice.body")}>
      <EnrollPanel state={enroll.state} consent={consent} onConsent={setConsent} modelNote={modelReady ? null : voiceDownloading ? "downloading" : strictOffline ? "offline" : "missing"} />
      <StepActions
        nav={nav}
        start={
          phase === "reading" ? (
            <Button size="lg" variant="primary" icon="stop" onClick={() => void enroll.finish()}>
              {t("onboarding.voice.stop")}
            </Button>
          ) : (
            (phase === "idle" || phase === "starting") && (
              <Button size="lg" variant="primary" icon="mic" disabled={!consent || !modelReady || phase === "starting"} onClick={() => void enroll.start()}>
                {error ? t("onboarding.voice.again") : t("onboarding.voice.start")}
              </Button>
            )
          )
        }
        primary={phase === "done" ? t("common.continue") : t("onboarding.skipForNow")}
        primaryProps={{ variant: phase === "done" ? "primary" : "ghost", className: phase === "done" ? undefined : "text-muted", disabled: phase === "saving" }}
        enter={phase === "done"}
      />
    </StepFrame>
  );
}
