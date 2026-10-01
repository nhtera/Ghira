// SPDX-License-Identifier: Apache-2.0
// D1 last step: the shortcuts, and the recording-consent explainer (RT-14: it
// explains, it does not give legal advice; the copyable message is the same
// one the live screen offers).
import { useTranslation } from "react-i18next";
import { Button, Icon, shortcutLabel, useToast, usePlatform } from "@ghi/ui";
import { SHORTCUTS } from "../../shell/shortcuts";
import { StepActions, StepFrame, type StepNav } from "./step-frame";

export function DoneStep({ nav, finishing }: { nav: StepNav; finishing?: boolean }) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const { show } = useToast();

  const copyConsent = async () => {
    try {
      await navigator.clipboard.writeText(t("live.consent.message"));
      show({ tone: "success", title: t("live.consent.copied") });
    } catch {
      show({ tone: "warning", title: t("system.commandFailed", { message: t("live.consent.copy") }) });
    }
  };

  const keys = [
    { chord: SHORTCUTS.toggleRecording, label: t("settings.shortcuts.startStop") },
    { chord: SHORTCUTS.mark, label: t("settings.shortcuts.markMoment") },
  ];

  return (
    <StepFrame serif title={t("onboarding.done.title")} body={t("onboarding.done.body")}>
      <Icon name="task_alt" size={44} className="-order-1 text-accent" />
      <ul className="m-0 flex list-none flex-col gap-2 p-0">
        {keys.map((k) => (
          <li key={k.chord} className="flex items-center gap-3">
            <kbd className="text-mono rounded-[10px] border border-line2 bg-surface2 px-3.5 py-2 text-[16px]">{shortcutLabel(k.chord, platform)}</kbd>
            <span className="text-[13px] text-muted">{k.label}</span>
          </li>
        ))}
      </ul>

      <section aria-labelledby="ob-consent-title" className="flex flex-col gap-2 rounded-xl border border-line bg-surface px-4 py-3.5">
        <h2 id="ob-consent-title" className="m-0 flex items-center gap-2 text-[14px] font-semibold">
          <Icon name="info" size={17} className="text-muted" />
          {t("onboarding.consent.title")}
        </h2>
        <p className="m-0 text-[13px] leading-normal text-muted">{t("onboarding.consent.body")}</p>
        <Button size="sm" variant="secondary" icon="content_copy" className="self-start" onClick={() => void copyConsent()}>
          {t("live.consent.copy")}
        </Button>
      </section>

      <StepActions nav={nav} primary={t("onboarding.done.openLibrary")} onPrimary={nav.finish} primaryProps={{ disabled: finishing }} />
    </StepFrame>
  );
}
