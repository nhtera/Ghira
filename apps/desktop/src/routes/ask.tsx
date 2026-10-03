// SPDX-License-Identifier: Apache-2.0
// D9: Ask across meetings. `?meeting=<gid>` adds the "This meeting" scope.
import { useSearch } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { AskScreenBody } from "../features/ask/ask-screen";

export function AskScreen() {
  const { t } = useTranslation();
  const { meeting } = useSearch({ from: "/shell/ask" });
  return (
    <AskScreenBody key={meeting ?? ""} meeting={meeting} title={t("nav.ask")} subtitle={t("ask.subtitle")} />
  );
}
