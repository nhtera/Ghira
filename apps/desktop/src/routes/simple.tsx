// SPDX-License-Identifier: Apache-2.0
// Screens whose content arrives later (People: phase
// 14c): the frame with their real title and subtitle.
import { useTranslation } from "react-i18next";
import { usePlatform } from "@ghi/ui";
import { Page } from "../shell/page";

export function PeopleScreen() {
  const { t } = useTranslation();
  return <Page title={t("nav.people")} subtitle={t("people.subtitle", { context: usePlatform(), people: 0, profiles: 0 })} />;
}
