// SPDX-License-Identifier: Apache-2.0
// The heading of a day group ("Today", "Thursday", "September 2026"), in the UI language.
import { useTranslation } from "react-i18next";
import type { DayGroupKey } from "./group-by-day";

export function useGroupTitle() {
  const { t, i18n } = useTranslation();
  const tag = i18n.language === "vi" ? "vi-VN" : "en-US";
  return (k: DayGroupKey): string => {
    switch (k.kind) {
      case "today":
        return t("common.today");
      case "yesterday":
        return t("common.yesterday");
      case "lastWeek":
        return t("library.groups.lastWeek");
      case "weekday":
        // 2024-01-07 was a Sunday: map getDay() onto a known date.
        return new Intl.DateTimeFormat(tag, { weekday: "long" }).format(new Date(2024, 0, 7 + k.day));
      case "month":
        return new Intl.DateTimeFormat(tag, {
          month: "long",
          year: "numeric",
        }).format(new Date(k.year, k.month, 1));
      case "unknown":
        return "";
    }
  };
}
