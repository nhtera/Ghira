// SPDX-License-Identifier: Apache-2.0
// /onboarding/$step: reads the settings, keeps the step in the URL, and on
// finish records `onboardingDone` (the other settings keep their value).
import { useNavigate, useParams } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { useToast } from "@ghi/ui";
import { OnboardingFlow } from "../features/onboarding/onboarding-flow";
import { resolveStep, type StepId } from "../features/onboarding/steps";
import type { MeetingLanguage } from "../bindings";
import { ipc } from "../ipc";
import { settingsQuery } from "../shell/root-view";

export function OnboardingScreen() {
  const { t } = useTranslation();
  const { show } = useToast();
  const navigate = useNavigate();
  const { step: raw } = useParams({ strict: false }) as { step?: string };
  const settings = useQuery(settingsQuery);
  const queryClient = useQueryClient();
  if (!settings.data) return null;
  const voiceEnabled = settings.data.voiceProfilesMe;
  const step = resolveStep(raw, voiceEnabled);

  const patch = async (p: { onboardingDone?: boolean; meetingLanguage?: MeetingLanguage }) => {
    const r = await ipc.commands.updateSettings({ onboardingDone: null, detectMeetings: null, globalMarkShortcut: null, strictOffline: null, meetingLanguage: null, ...p });
    if (r.status === "error") {
      show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
      return false;
    }
    queryClient.setQueryData(settingsQuery.queryKey, r.data);
    return true;
  };
  const finish = async () => {
    if (await patch({ onboardingDone: true })) await navigate({ to: "/meetings" });
  };

  return (
    <OnboardingFlow
      step={step}
      onStep={(s: StepId) => void navigate({ to: "/onboarding/$step", params: { step: s } })}
      onFinish={finish}
      voiceEnabled={voiceEnabled}
      strictOffline={settings.data.strictOffline}
      language={settings.data.meetingLanguage}
      onLanguage={(l) => void patch({ meetingLanguage: l })}
    />
  );
}
