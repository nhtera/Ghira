// SPDX-License-Identifier: Apache-2.0
// M1 last step: nothing records on its own, say so, summarize what was set
// up, and go.
import { ListRow, ListSection, PhoneButton } from "@ghi/ui";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingLanguage, ProcessingTarget } from "../../bindings";
import { ipc } from "../../ipc";
import { modelsView } from "./models-model";
import { StepLayout } from "./step-layout";

type Summary = { language: MeetingLanguage; target: ProcessingTarget; modelsReady: boolean; voice: boolean };

const LANGUAGE = { en: "mobile.onboarding.languages.en", vi: "mobile.onboarding.languages.vi", auto: "mobile.onboarding.languages.both" } as const;
const TARGET = { phone: "mobile.target.phone", desktop: "mobile.target.desktop", cloud: "mobile.target.cloud" } as const;

export function DoneStep({ onNext }: { onNext: () => void }) {
  const { t } = useTranslation();
  const [summary, setSummary] = useState<Summary | null>(null);

  useEffect(() => {
    let alive = true;
    void (async () => {
      const [app, mobile, models, voice] = await Promise.all([ipc.commands.getSettings(), ipc.commands.mobileSettings(), ipc.commands.modelsStatus(), ipc.commands.voiceStatus()]);
      if (!alive) return;
      setSummary({
        language: app.status === "ok" ? app.data.meetingLanguage : "auto",
        target: mobile.status === "ok" ? mobile.data.defaultTarget : "phone",
        modelsReady: models.status === "ok" && modelsView(models.data).allReady,
        voice: voice.status === "ok" && voice.data.meProfile !== null,
      });
    })();
    return () => {
      alive = false;
    };
  }, []);

  return (
    <StepLayout
      icon="task_alt"
      title={t("mobile.onboarding.done.title")}
      subtitle={t("mobile.onboarding.done.body")}
      footer={<PhoneButton onClick={onNext}>{t("mobile.onboarding.done.start")}</PhoneButton>}
    >
      {summary && (
        <ListSection className="mx-0 my-0">
          <ListRow title={t("mobile.onboarding.done.summary.languages")} value={t(LANGUAGE[summary.language])} />
          <ListRow title={t("mobile.onboarding.done.summary.processing")} value={t(TARGET[summary.target])} />
          <ListRow title={t("mobile.onboarding.done.summary.models")} value={summary.modelsReady ? t("mobile.onboarding.models.state.ready") : t("mobile.onboarding.models.state.missing")} />
          <ListRow title={t("mobile.onboarding.done.summary.voice")} value={summary.voice ? t("mobile.voice.saved") : t("mobile.onboarding.done.summary.voiceNone")} />
        </ListSection>
      )}
    </StepLayout>
  );
}
