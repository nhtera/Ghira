// SPDX-License-Identifier: Apache-2.0
// M1 step 4: recording other people needs their agreement in many places. The
// app reminds before every recording (M2); this says so once, up front.
import { Icon, type IconName } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import { PhoneButton } from "../record/phone-button";
import { StepLayout } from "./step-layout";

const ITEMS: { icon: IconName; key: "tell" | "calls" | "local" }[] = [
  { icon: "record_voice_over", key: "tell" },
  { icon: "phone_disabled", key: "calls" },
  { icon: "lock", key: "local" },
];

export function ConsentStep({ onNext }: { onNext: () => void }) {
  const { t } = useTranslation();
  return (
    <StepLayout
      icon="verified_user"
      title={t("mobile.onboarding.consent.title")}
      subtitle={t("mobile.onboarding.consent.body")}
      footer={
        <PhoneButton onClick={onNext}>
          {t("mobile.common.continue")}
        </PhoneButton>
      }
    >
      <ul className="m-0 flex list-none flex-col gap-3 p-0">
        {ITEMS.map((i) => (
          <li key={i.key} className="text-ios-subhead flex items-start gap-3">
            <Icon
              name={i.icon}
              size={22}
              className="mt-0.5 size-[1.375rem] shrink-0 text-accent"
            />
            {t(`mobile.onboarding.consent.items.${i.key}`)}
          </li>
        ))}
      </ul>
    </StepLayout>
  );
}
