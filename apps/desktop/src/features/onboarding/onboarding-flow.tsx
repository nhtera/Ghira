// SPDX-License-Identifier: Apache-2.0
// D1 onboarding: a step rail and one step at a time. The flow owns what must
// outlive a step (the model download); the route owns the
// URL and what "finish" does.
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { APP_NAME } from "@ghi/i18n";
import { Icon, cn, usePlatform } from "@ghi/ui";
import { DoneStep } from "./done-step";
import type { MeetingLanguage } from "../../bindings";
import { LanguagesStep } from "./languages-step";
import { ModelsStep } from "./models-step";
import { PermissionsStep } from "./permissions-step";
import { RecoveryStep } from "./recovery-step";
import { flowSteps, nextStep, prevStep, type StepId } from "./steps";
import type { StepNav } from "./step-frame";
import { TestStep } from "./test-step";
import { useModelDownload } from "./use-model-download";
import { VoiceStep } from "./voice-step";
import { WelcomeStep } from "./welcome-step";

export type OnboardingFlowProps = {
  step: StepId;
  onStep: (s: StepId) => void;
  /** Mark onboarding done and leave. */
  onFinish: () => void | Promise<void>;
  /** `settings.voiceProfilesMe`: the "Your voice" step is skipped while false. */
  voiceEnabled: boolean;
  strictOffline: boolean;
  /** The default meeting language, and its change (the route stores it). */
  language: MeetingLanguage;
  onLanguage: (l: MeetingLanguage) => void;
  /** Test recording length; tests shorten it. */
  testSeconds?: number;
};

const INTERACTIVE = "button, a, input, textarea, select, [role=radio], [role=checkbox]";

export function OnboardingFlow({ step, onStep, onFinish, voiceEnabled, strictOffline, language, onLanguage, testSeconds }: OnboardingFlowProps) {
  const { t } = useTranslation();
  const context = usePlatform();
  const steps = useMemo(() => flowSteps(voiceEnabled), [voiceEnabled]);
  const index = steps.indexOf(step);
  const [finishing, setFinishing] = useState(false);
  const dl = useModelDownload();
  const content = useRef<HTMLDivElement>(null);

  const nav: StepNav = {
    next: () => onStep(nextStep(step, voiceEnabled)),
    back: () => onStep(prevStep(step, voiceEnabled)),
    finish: () => {
      if (finishing) return;
      setFinishing(true);
      void Promise.resolve(onFinish()).finally(() => setFinishing(false));
    },
    canGoBack: index > 0,
  };

  // Enter continues (unless a control has focus and takes it); Esc does not
  // close: onboarding is a place, not a dialog.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") return e.preventDefault();
      if (e.key !== "Enter" || e.repeat || e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
      if ((e.target as Element | null)?.closest?.(INTERACTIVE)) return;
      const primary = content.current?.querySelector<HTMLButtonElement>("[data-onboarding-primary]");
      if (primary && !primary.disabled) {
        e.preventDefault();
        primary.click();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  // A new step puts the focus on its heading (not on the first mount).
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    content.current?.querySelector<HTMLElement>("[data-onboarding-title]")?.focus();
  }, [step]);

  return (
    <div className="grid h-screen print:hidden min-h-0 grid-cols-[232px_minmax(0,1fr)] bg-surface">
      <aside className="flex min-h-0 flex-col gap-1 overflow-y-auto border-r border-line bg-surface2 px-5 py-7">
        <div data-tauri-drag-region className="mb-5 flex items-center gap-2.5">
          <span aria-hidden className="grid size-8 place-items-center rounded-lg bg-accent font-serif text-[20px] font-semibold text-on-accent">
            {APP_NAME.charAt(0).toLowerCase()}
          </span>
          <b className="text-[16px]">{APP_NAME}</b>
        </div>
        <nav aria-label={t("onboarding.progress")}>
          <ol className="m-0 flex list-none flex-col gap-0.5 p-0">
            {steps.map((s, i) => {
              const current = i === index;
              const done = i < index;
              return (
                <li key={s} aria-current={current ? "step" : undefined} className={cn("flex h-9 items-center gap-2.5 text-[13.5px]", current ? "font-semibold text-ink" : done ? "text-muted" : "text-faint")}>
                  <span
                    aria-hidden
                    className={cn(
                      "grid size-[22px] flex-none place-items-center rounded-full border-[1.5px] text-[11px] font-bold",
                      current ? "border-accent bg-accent text-on-accent" : done ? "border-accent bg-accent-soft text-accent" : "border-line2",
                    )}
                  >
                    {done ? <Icon name="check" size={14} /> : i + 1}
                  </span>
                  <span className="min-w-0">{t(`onboarding.steps.${s}`)}</span>
                </li>
              );
            })}
          </ol>
        </nav>
        <div className="flex-1" />
        <div className="flex gap-2 text-[12px] text-muted">
          <Icon name="lock" size={16} className="flex-none text-accent" />
          {t(`live.localLine_${context}`)}
        </div>
      </aside>

      <main ref={content} className="min-h-0 overflow-y-auto">
        <div data-tauri-drag-region className="h-7" />
        <div key={step} className="px-12 pt-8 pb-16 xl:px-[72px] xl:pt-12">
          <p className="sr-only">{t("onboarding.stepOf", { step: index + 1, total: steps.length })}</p>
          {step === "welcome" && <WelcomeStep nav={nav} />}
          {step === "languages" && <LanguagesStep nav={nav} initial={language} onSave={onLanguage} />}
          {step === "models" && <ModelsStep nav={nav} dl={dl} strictOffline={strictOffline} />}
          {step === "permissions" && <PermissionsStep nav={nav} />}
          {step === "voice" && <VoiceStep nav={nav} />}
          {step === "test" && <TestStep nav={nav} seconds={testSeconds} />}
          {step === "recovery" && <RecoveryStep nav={nav} />}
          {step === "done" && <DoneStep nav={nav} finishing={finishing} />}
        </div>
      </main>
    </div>
  );
}
