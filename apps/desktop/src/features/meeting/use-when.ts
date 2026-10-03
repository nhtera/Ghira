// SPDX-License-Identifier: Apache-2.0
// "Today · 10:02", "Yesterday · 09:15", or the date for anything older.
import { formatDate, formatTime, type Locale } from "@ghi/i18n";
import { useTranslation } from "react-i18next";

export function useWhen(): (ms: number) => string {
  const { t, i18n } = useTranslation();
  const locale: Locale = i18n.language === "vi" ? "vi" : "en";
  return (ms) => {
    const day = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
    const ago = Math.round((day(new Date()) - day(new Date(ms))) / 86_400_000);
    if (ago === 0 || ago === 1) return `${t(ago === 0 ? "common.today" : "common.yesterday")} · ${formatTime(ms, locale)}`;
    return `${formatDate(ms, locale)} · ${formatTime(ms, locale)}`;
  };
}
