// SPDX-License-Identifier: Apache-2.0
// M1 step 2: say why the microphone is needed before the system asks (priming),
// then handle the answer. Denied is a state with a way out: Settings, or on
// without it (recording stays off until it is allowed).
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { MicPermission } from "../../bindings";
import { ipc } from "../../ipc";
import { PhoneButton } from "@ghi/ui";
import { StepLayout } from "./step-layout";

export function MicStep({ onNext }: { onNext: () => void }) {
  const { t } = useTranslation();
  const [mic, setMic] = useState<MicPermission | null>(null);

  useEffect(() => {
    let alive = true;
    const read = () =>
      void ipc.commands.micPermission().then(
        (m) => alive && setMic(m),
        () => alive && setMic("notDetermined"),
      );
    read();
    // Back from Settings with the switch on.
    const onVisible = () => document.visibilityState === "visible" && read();
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      alive = false;
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, []);

  if (mic === null) return null;

  if (mic === "granted") {
    return (
      <StepLayout
        icon="check_circle"
        title={t("mobile.onboarding.mic.allowed")}
        footer={
          <PhoneButton onClick={onNext}>
            {t("mobile.common.continue")}
          </PhoneButton>
        }
      />
    );
  }

  if (mic === "denied") {
    return (
      <StepLayout
        icon="mic_off"
        title={t("mobile.onboarding.mic.title")}
        subtitle={t("mobile.onboarding.mic.denied")}
        footer={
          <>
            <PhoneButton onClick={() => void ipc.commands.openAppSettings()}>
              {t("mobile.common.openSettings")}
            </PhoneButton>
            <PhoneButton variant="secondary" onClick={onNext}>
              {t("mobile.common.notNow")}
            </PhoneButton>
          </>
        }
      />
    );
  }

  return (
    <StepLayout
      icon="mic"
      title={t("mobile.onboarding.mic.title")}
      subtitle={t("mobile.onboarding.mic.body")}
      footer={
        <PhoneButton
          onClick={() =>
            void ipc.commands
              .requestMicPermission()
              .then(setMic, () => setMic("denied"))
          }
        >
          {t("mobile.onboarding.mic.allow")}
        </PhoneButton>
      }
    />
  );
}
