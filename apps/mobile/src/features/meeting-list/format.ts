// SPDX-License-Identifier: Apache-2.0
// Locale-aware labels shared by the meetings list, the meeting view and search.
import { formatTime, type Locale } from "@ghi/i18n";
import { useTranslation } from "react-i18next";
import { minutesOf, type DayKey } from "./group";

export function useLocale(): Locale {
  return useTranslation().i18n.language === "vi" ? "vi" : "en";
}

/** "Today", "Yesterday" or "Saturday, 12 September 2026". */
export function useDayLabel() {
  const { t } = useTranslation();
  const locale = useLocale();
  return (day: DayKey) => {
    if (day === "today") return t("mobile.meetings.today");
    if (day === "yesterday") return t("mobile.meetings.yesterday");
    return new Intl.DateTimeFormat(locale === "vi" ? "vi-VN" : "en-US", {
      weekday: "long",
      day: "numeric",
      month: "long",
      year: "numeric",
    }).format(day.date);
  };
}

/** "9:30 AM · 42 min" (the time part only when known). */
export function useMeetingWhen() {
  const { t } = useTranslation();
  const locale = useLocale();
  return (startedAt: number | null, durationMs: number | null) => {
    const minutes = minutesOf(durationMs);
    const length =
      minutes === null
        ? null
        : minutes < 1
          ? t("mobile.meetings.durationShort")
          : t("mobile.meetings.duration", { minutes });
    return [startedAt === null ? null : formatTime(startedAt, locale), length]
      .filter(Boolean)
      .join(" · ");
  };
}
