// SPDX-License-Identifier: Apache-2.0
import { useTranslation } from "react-i18next";
import { Icon, usePlatform } from "@ghi/ui";
import { StepActions, StepFrame, type StepNav } from "./step-frame";

export function WelcomeStep({ nav }: { nav: StepNav }) {
  const { t } = useTranslation();
  const context = usePlatform();
  const promises = [t("onboarding.welcome.promises.onDevice", { context }), t("onboarding.welcome.promises.noCloud"), t("onboarding.welcome.promises.citations")];
  return (
    <StepFrame serif title={t("onboarding.welcome.title", { context })} body={t("onboarding.welcome.body")}>
      <ul className="m-0 mt-1 flex list-none flex-col gap-3 p-0">
        {promises.map((p) => (
          <li key={p} className="flex items-start gap-3 text-[14.5px] leading-relaxed">
            <Icon name="check_circle" size={20} className="mt-0.5 flex-none text-accent" />
            {p}
          </li>
        ))}
      </ul>
      <StepActions nav={nav} primary={t("onboarding.getStarted")} skip={t("onboarding.skipForNow")} onSkip={nav.finish} />
    </StepFrame>
  );
}
