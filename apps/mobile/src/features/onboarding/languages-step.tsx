// SPDX-License-Identifier: Apache-2.0
// M1 step 1: the language(s) spoken in meetings. Stored as the default meeting
// language; "both" is code-switching ("auto").
import { Icon, cn } from "@ghi/ui";
import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingLanguage } from "../../bindings";
import { ipc } from "../../ipc";
import { PhoneButton } from "../record/phone-button";
import { StepLayout } from "./step-layout";

const OPTIONS: {
  value: MeetingLanguage;
  label:
    | "mobile.onboarding.languages.en"
    | "mobile.onboarding.languages.vi"
    | "mobile.onboarding.languages.both";
}[] = [
  { value: "en", label: "mobile.onboarding.languages.en" },
  { value: "vi", label: "mobile.onboarding.languages.vi" },
  { value: "auto", label: "mobile.onboarding.languages.both" },
];

export function LanguagesStep({ onNext }: { onNext: () => void }) {
  const { t } = useTranslation();
  const name = useId();
  const [value, setValue] = useState<MeetingLanguage>("auto");

  useEffect(() => {
    let alive = true;
    void ipc.commands
      .getSettings()
      .then(
        (r) => alive && r.status === "ok" && setValue(r.data.meetingLanguage),
      );
    return () => {
      alive = false;
    };
  }, []);

  const next = async () => {
    await ipc.commands.updateSettings({ meetingLanguage: value });
    onNext();
  };

  return (
    <StepLayout
      icon="language"
      title={t("mobile.onboarding.languages.title")}
      subtitle={t("mobile.onboarding.languages.subtitle")}
      footer={
        <PhoneButton onClick={() => void next()}>
          {t("mobile.common.continue")}
        </PhoneButton>
      }
    >
      <fieldset className="m-0 flex min-w-0 flex-col gap-2 border-0 p-0">
        <legend className="sr-only">
          {t("mobile.onboarding.languages.title")}
        </legend>
        {OPTIONS.map((o) => (
          <label
            key={o.value}
            className={cn(
              "text-ios-body flex min-h-ios-target cursor-pointer items-center gap-3 rounded-(--ios-radius-group) border-2 border-transparent bg-surface px-4 py-3",
              "has-checked:border-accent has-checked:bg-accent-soft has-focus-visible:outline-2 has-focus-visible:outline-offset-2 has-focus-visible:outline-accent",
            )}
          >
            <input
              type="radio"
              name={name}
              value={o.value}
              checked={value === o.value}
              onChange={() => setValue(o.value)}
              className="sr-only"
            />
            <span className="flex-1">{t(o.label)}</span>
            <Icon
              name={
                value === o.value
                  ? "radio_button_checked"
                  : "radio_button_unchecked"
              }
              size={24}
              className={cn(
                "size-6 shrink-0",
                value === o.value ? "text-accent" : "text-faint",
              )}
            />
          </label>
        ))}
      </fieldset>
    </StepLayout>
  );
}
