// SPDX-License-Identifier: Apache-2.0
// D8: People. The list is Me first; the voice and name actions are on a
// person's page.
import { useTranslation } from "react-i18next";
import { usePlatform } from "@ghi/ui";
import { PeopleBody } from "../features/people/people-screen";
import { usePeople } from "../features/people/queries";

export function PeopleScreen() {
  const { t } = useTranslation();
  const platform = usePlatform();
  const q = usePeople();
  const people = q.data?.people ?? [];
  const profiles = people.filter((p) => p.voice.kind !== "none").length;
  const subtitle = q.data
    ? t("people.summary", { context: platform, people: t("people.peopleCount", { count: people.length }), profiles: t("people.profilesCount", { count: profiles }) })
    : undefined;
  return <PeopleBody title={t("nav.people")} subtitle={subtitle} />;
}
