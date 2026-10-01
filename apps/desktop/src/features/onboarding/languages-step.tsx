// SPDX-License-Identifier: Apache-2.0
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Icon, cn } from "@ghi/ui";
import type { MeetingLanguage } from "../../bindings";
import { StepActions, StepFrame, type StepNav } from "./step-frame";

// "auto" is both languages, code-switching inside one sentence.
const OPTIONS: MeetingLanguage[] = ["en", "vi", "auto"];

/**
 * The default language of new meetings (`settings.meetingLanguage`). The
 * choice is saved once, on Continue (arrow keys would otherwise send one
 * request per press, and replies could land out of order).
 */
export function LanguagesStep({ nav, initial, onSave }: { nav: StepNav; initial: MeetingLanguage; onSave: (l: MeetingLanguage) => void }) {
  const { t } = useTranslation();
  const [value, onChange] = useState(initial);
  const label: Record<MeetingLanguage, string> = {
    en: t("onboarding.languages.english"),
    vi: t("onboarding.languages.vietnamese"),
    auto: t("onboarding.languages.both"),
  };
  return (
    <StepFrame title={t("onboarding.languages.title")} body={t("onboarding.languages.body")}>
      <div role="radiogroup" aria-label={t("onboarding.languages.title")} className="flex flex-col gap-2">
        {OPTIONS.map((o) => {
          const on = o === value;
          return (
            <button
              key={o}
              type="button"
              role="radio"
              aria-checked={on}
              tabIndex={on ? 0 : -1}
              onClick={() => onChange(o)}
              onKeyDown={(e) => {
                const d = e.key === "ArrowDown" || e.key === "ArrowRight" ? 1 : e.key === "ArrowUp" || e.key === "ArrowLeft" ? -1 : 0;
                if (!d) return;
                e.preventDefault();
                const next = OPTIONS[(OPTIONS.indexOf(o) + d + OPTIONS.length) % OPTIONS.length]!;
                onChange(next);
                (e.currentTarget.parentElement?.querySelector(`[data-lang="${next}"]`) as HTMLElement | null)?.focus();
              }}
              data-lang={o}
              className={cn(
                "flex h-[52px] items-center gap-2.5 rounded-xl border-[1.5px] px-4 text-left text-[15px] font-medium text-ink",
                on ? "border-accent bg-accent-soft" : "border-line2 bg-surface hover:bg-surface2",
              )}
            >
              <Icon name="translate" size={20} className={on ? "text-accent" : "text-muted"} />
              <span className="flex-1">{label[o]}</span>
              {on && <Icon name="check_circle" size={18} className="text-accent" />}
            </button>
          );
        })}
      </div>
      <StepActions
        nav={nav}
        onPrimary={() => {
          if (value !== initial) onSave(value);
          nav.next();
        }}
      />
    </StepFrame>
  );
}
