// SPDX-License-Identifier: Apache-2.0
// Popover "Next" row above Recent: the next event and the same toggle (phase 14d).
import { useTranslation } from "react-i18next";
import type { Locale } from "@ghi/i18n";
import { Icon } from "@ghi/ui";
import { useNow } from "../live/clock";
import { AskToggle } from "./toggle";
import {
  eventTitle,
  isMeetingLike,
  joinAppName,
  useUpcoming,
  whenLabel,
} from "./use-calendar";

export function NextEvent() {
  const { t, i18n } = useTranslation();
  const locale: Locale = i18n.language === "vi" ? "vi" : "en";
  const { data } = useUpcoming(10);
  const now = useNow(true);
  const event = data?.find(isMeetingLike);
  if (!event) return null;
  const app = joinAppName(event.joinApp);
  return (
    <section aria-label={t("tray.next")} className="flex items-center gap-2.5 border-b border-line px-1.5 py-2">
      <h2 className="sr-only">{t("tray.next")}</h2>
      <Icon name="event_upcoming" size={18} className="flex-none text-muted" />
      <div className="min-w-0 flex-1">
        <div className="text-[11.5px] text-faint">
          {t("tray.next")}
          {t("common.metaSep")}
          <span>{whenLabel(event, now, t, locale)}</span>
          {app && `${t("common.metaSep")}${app}`}
        </div>
        <div className="truncate text-[13px] font-medium">{eventTitle(t, event.title)}</div>
      </div>
      <AskToggle event={event} compact />
    </section>
  );
}
