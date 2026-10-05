// SPDX-License-Identifier: Apache-2.0
// M1: the first-launch steps as a paged flow, full screen: a progress bar per
// step on top, the next page sliding in from the right (no slide under reduced
// motion), a swipe to the right or the back chevron to go back. Resumes at the
// first step not completed; a step completes when the user moves on (skipping
// counts), so there is no swipe forward. The last step ends onboarding and
// opens Record.
import { Banner, cn, Icon, PhoneButton } from "@ghi/ui";
import { useNavigate } from "@tanstack/react-router";
import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type ComponentType,
} from "react";
import { useTranslation } from "react-i18next";
import type { OnboardingStep } from "../../bindings";
import { ipc } from "../../ipc";
import { ConsentStep } from "./consent-step";
import { DoneStep } from "./done-step";
import { LanguagesStep } from "./languages-step";
import { MicStep } from "./mic-step";
import { ModelsStep } from "./models-step";
import { ProcessingStep } from "./processing-step";
import { PairStep } from "./pair-step";
import { resumeStep, STEPS, stepsFor } from "./state";
import { VoiceStep } from "./voice-step";

const SCREENS: Record<OnboardingStep, ComponentType<{ onNext: () => void }>> = {
  languages: LanguagesStep,
  micPriming: MicStep,
  voice: VoiceStep,
  consent: ConsentStep,
  pair: PairStep,
  processing: ProcessingStep,
  models: ModelsStep,
  done: DoneStep,
};

export function OnboardingFlow() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [index, setIndex] = useState<number | null>(null);
  // The steps depend on the core: pairing is a step only when it is available.
  const [steps, setSteps] = useState<readonly OnboardingStep[]>(STEPS);
  // The setup could not be read or saved: say so, never start over silently.
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [dir, setDir] = useState<"next" | "prev" | null>(null);
  const root = useRef<HTMLElement>(null);
  const stepId = useId();
  const touch = useRef<{ x: number; y: number } | null>(null);

  useEffect(() => {
    let alive = true;
    void ipc.commands.onboardingState().then(
      (r) => {
        if (!alive) return;
        if (r.status === "ok") {
          const list = stepsFor(r.data.syncAvailable);
          setFailed(false);
          setSteps(list);
          setIndex(list.indexOf(resumeStep(r.data)));
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
    if (index === null) return;
    const title = root.current?.querySelector<HTMLElement>("[data-step-title]");
    // "Step N of M" is read with the title when the page changes.
    title?.setAttribute("aria-describedby", stepId);
    title?.focus({ preventScroll: true });
  }, [index, stepId]);

  const next = useCallback(async () => {
    if (index === null) return;
    const step = steps[index];
    const saved = await ipc.commands.onboardingCompleteStep(step).catch(() => null);
    if (saved?.status !== "ok") return setFailed(true);
    setFailed(false);
    if (step === "done") {
      await ipc.commands.updateSettings({ onboardingDone: true });
      void navigate({ to: "/record" });
    } else {
      setDir("next");
      setIndex(index + 1);
    }
  }, [index, navigate, steps]);

  const back = useCallback(() => {
    setDir("prev");
    setIndex((i) => (i === null || i === 0 ? i : i - 1));
  }, []);

  if (index === null && !failed) return null;
  if (index === null) {
    return (
      <main data-screen="onboarding" data-step="error" className="flex h-full flex-col justify-center gap-4 px-5 pt-[env(safe-area-inset-top)] pb-safe">
        <h1 className="text-ios-title2 m-0">{t("mobile.onboarding.loadError")}</h1>
        <PhoneButton onClick={() => setAttempt(attempt + 1)}>{t("mobile.onboarding.models.retry")}</PhoneButton>
      </main>
    );
  }
  const Screen = SCREENS[steps[index]];
  return (
    <main ref={root} data-screen="onboarding" data-step={steps[index]} className="flex h-full flex-col overflow-hidden pt-[env(safe-area-inset-top)]">
      <p id={stepId} className="sr-only">{t("mobile.onboarding.step", { current: index + 1, total: steps.length })}</p>
      <div aria-hidden="true" data-testid="onboarding-progress" className="flex gap-1.5 px-6 pt-4">
        {steps.map((s, i) => (
          <span key={s} data-done={i <= index} className={cn("h-1 flex-1 rounded-sm", i <= index ? "bg-accent" : "bg-line2")} />
        ))}
      </div>
      <div className="flex min-h-ios-target items-center px-3">
        {index > 0 && (
          <button type="button" onClick={back} aria-label={t("mobile.nav.back")} className="grid min-h-ios-target min-w-ios-target place-items-center text-accent">
            <Icon name="chevron_left" size={28} className="size-7" />
          </button>
        )}
      </div>
      {failed && <Banner variant="warning" title={t("mobile.onboarding.loadError")} className="mx-6" />}
      <div
        key={steps[index]}
        data-testid="onboarding-page"
        onTouchStart={(e) => {
          const p = e.touches[0];
          touch.current = p ? { x: p.clientX, y: p.clientY } : null;
        }}
        onTouchEnd={(e) => {
          const from = touch.current;
          const p = e.changedTouches[0];
          touch.current = null;
          if (!from || !p || index === 0) return;
          const dx = p.clientX - from.x;
          if (dx > 72 && Math.abs(p.clientY - from.y) < dx / 2) back();
        }}
        className={cn("flex min-h-0 flex-1 flex-col", dir === "next" && "ios-page-next", dir === "prev" && "ios-page-prev")}
      >
        <Screen onNext={() => void next()} />
      </div>
    </main>
  );
}
