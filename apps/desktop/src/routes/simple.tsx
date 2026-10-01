// SPDX-License-Identifier: Apache-2.0
// Screens whose content arrives in phases 10–11 (People, Ask, Import,
// meeting detail, onboarding): the frame with their real title and subtitle.
import { useParams } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { usePlatform } from "@ghi/ui";
import { Page } from "../shell/page";

export function PeopleScreen() {
  const { t } = useTranslation();
  return <Page title={t("nav.people")} subtitle={t("people.subtitle", { context: usePlatform(), people: 0, profiles: 0 })} />;
}

export function AskScreen() {
  const { t } = useTranslation();
  return <Page title={t("nav.ask")} subtitle={t("ask.subtitle")} />;
}

export function ImportScreen() {
  const { t } = useTranslation();
  return <Page title={t("nav.import")} subtitle={t("import.subtitle", { context: usePlatform() })} />;
}

export function MeetingDetailScreen() {
  const { t } = useTranslation();
  const { tab } = useParams({ from: "/meetings/$id/$tab" });
  return <Page title={tab === "transcript" ? t("notes.transcriptTab") : t("notes.tab")} />;
}

export function OnboardingScreen() {
  const { t } = useTranslation();
  return <Page title={t("onboarding.steps.welcome")} />;
}
