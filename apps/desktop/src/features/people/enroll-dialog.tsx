// SPDX-License-Identifier: Apache-2.0
// "Record your voice again": the enrollment in a dialog, from People and from
// Settings → Privacy. The new profile replaces the old one. Closing the
// dialog releases the mic (the body unmounts).
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Dialog, useToast } from "@ghi/ui";
import { EnrollPanel, useConsentKey } from "./enroll-panel";
import { useInvalidatePeople, useVoiceStatus } from "./queries";
import { useVoiceEnroll } from "./use-voice-enroll";

function Body({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const { show } = useToast();
  const key = useConsentKey();
  const invalidate = useInvalidatePeople();
  const voice = useVoiceStatus({ pollWhileNotReady: true });
  const enroll = useVoiceEnroll(key, () => {
    invalidate();
    show({ tone: "success", title: t("consent.self.saved") });
  });
  const [consent, setConsent] = useState(false);
  const { phase, error } = enroll.state;
  const modelReady = voice.data?.modelReady ?? true;
  const done = phase === "done";
  return (
    <Dialog
      open
      onOpenChange={(o) => !o && onClose()}
      width={560}
      title={t("consent.self.title")}
      description={t("consent.self.body")}
      footer={
        <>
          <Button onClick={onClose}>{done ? t("common.continue") : t("common.cancel")}</Button>
          {phase === "reading" ? (
            <Button variant="primary" icon="stop" onClick={() => void enroll.finish()}>
              {t("onboarding.voice.stop")}
            </Button>
          ) : (
            !done && (
              <Button variant="primary" icon="mic" disabled={!consent || !modelReady || phase !== "idle"} onClick={() => void enroll.start()}>
                {error ? t("onboarding.voice.again") : t("onboarding.voice.start")}
              </Button>
            )
          )}
        </>
      }
    >
      <div className="flex flex-col gap-3">
        <EnrollPanel state={enroll.state} consent={consent} onConsent={setConsent} modelNote={modelReady ? null : "missing"} />
      </div>
    </Dialog>
  );
}

export function EnrollDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  // Mounted only while open: each opening starts unticked and idle.
  return open ? <Body onClose={onClose} /> : null;
}
