// SPDX-License-Identifier: Apache-2.0
// M1: the first-launch steps, full screen. Resumes at the first step not
// completed; a step completes when the user moves on (skipping counts). The
// last step ends onboarding and opens Record.
import { Banner, Icon } from "@ghi/ui";
import { useNavigate } from "@tanstack/react-router";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ComponentType,
} from "react";
import { useTranslation } from "react-i18next";
import type { OnboardingStep } from "../../bindings";
import { ipc } from "../../ipc";
import { PhoneButton } from "../record/phone-button";
import { ConsentStep } from "./consent-step";
import { DoneStep } from "./done-step";
import { LanguagesStep } from "./languages-step";
import { MicStep } from "./mic-step";
import { ModelsStep } from "./models-step";
import { ProcessingStep } from "./processing-step";
import { resumeStep, STEPS } from "./state";
import { VoiceStep } from "./voice-step";

const SCREENS: Record<
  Exclude<OnboardingStep, "pair">,
  ComponentType<{ onNext: () => void }>
> = {
  languages: LanguagesStep,
  micPriming: MicStep,
  voice: VoiceStep,
  consent: ConsentStep,
  processing: ProcessingStep,
  models: ModelsStep,
  done: DoneStep,
};

export function OnboardingFlow() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [index, setIndex] = useState<number | null>(null);
  // The setup could not be read or saved: say so, never start over silently.
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const root = useRef<HTMLElement>(null);

  useEffect(() => {
    let alive = true;
    void ipc.commands.onboardingState().then(
      (r) => {
        if (!alive) return;
        if (r.status === "ok") {
          setFailed(false);
          setIndex(STEPS.indexOf(resumeStep(r.data)));
        } else setFailed(true);
      },
      () => alive && setFailed(true),
    );
    return () => {
      alive = false;
    };
  }, [attempt]);

  // A new step starts at its heading, for VoiceOver and the keyboard.
  useEffect(() => {
    if (index !== null) root.current?.querySelector<HTMLElement>("[data-step-title]")?.focus({ preventScroll: true });
  }, [index]);

  const next = useCallback(async () => {
    if (index === null) return;
    const step = STEPS[index];
    const saved = await ipc.commands.onboardingCompleteStep(step).catch(() => null);
    if (saved?.status !== "ok") return setFailed(true);
    setFailed(false);
    if (step === "done") {
      await ipc.commands.updateSettings({ onboardingDone: true });
      void navigate({ to: "/record" });
    } else setIndex(index + 1);
  }, [index, navigate]);

  if (index === null && !failed) return null;
  if (index === null) {
    return (
      <main data-screen="onboarding" data-step="error" className="flex h-full flex-col justify-center gap-4 px-5 pt-[env(safe-area-inset-top)] pb-safe">
        <h1 className="text-ios-title2 m-0">{t("mobile.onboarding.loadError")}</h1>
        <PhoneButton onClick={() => setAttempt(attempt + 1)}>{t("mobile.onboarding.models.retry")}</PhoneButton>
      </main>
    );
  }
  const Screen = SCREENS[STEPS[index] as keyof typeof SCREENS];
  return (
    <main ref={root} data-screen="onboarding" data-step={STEPS[index]} className="flex h-full flex-col pt-[env(safe-area-inset-top)]">
      <header className="flex min-h-ios-target items-center gap-2 px-2">
        <div className="min-w-ios-target">
          {index > 0 && (
            <button type="button" onClick={() => setIndex(index - 1)} aria-label={t("mobile.nav.back")} className="grid min-h-ios-target min-w-ios-target place-items-center text-accent">
              <Icon name="chevron_left" size={28} className="size-7" />
            </button>
          )}
        </div>
        <p className="text-ios-footnote m-0 flex-1 text-center text-muted">{t("mobile.onboarding.step", { current: index + 1, total: STEPS.length })}</p>
        <div className="min-w-ios-target" />
      </header>
      {failed && <Banner variant="warning" title={t("mobile.onboarding.loadError")} className="mx-5" />}
      <Screen key={STEPS[index]} onNext={() => void next()} />
    </main>
  );
}
