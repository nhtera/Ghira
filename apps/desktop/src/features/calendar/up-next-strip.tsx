// SPDX-License-Identifier: Apache-2.0
// Library "Up next" strip: the next meeting-like calendar event and its "ask to record when it starts" toggle (phase 14d).
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

export function UpNextStrip() {
  const { t, i18n } = useTranslation();
  const locale: Locale = i18n.language === "vi" ? "vi" : "en";
  const { data } = useUpcoming(10);
  const now = useNow(true);
  const event = data?.find(isMeetingLike);
  if (!event) return null;
  const app = joinAppName(event.joinApp);
  const when = [whenLabel(event, now, t, locale), app].filter(Boolean).join(t("common.metaSep"));
  return (
    <section
      aria-label={t("library.upNext")}
      className="mb-[18px] flex flex-none flex-wrap items-center gap-x-3 gap-y-2 rounded-row bg-accent-soft px-3.5 py-2.5"
    >
      <Icon name="event_upcoming" size={20} className="flex-none text-accent" />
      <p className="m-0 min-w-0 flex-1 truncate text-[13px]">
        <span className="font-semibold text-accent">
          {t("library.upNext")}
          {t("common.metaSep")}
          {when}
        </span>
        {t("common.metaSep")}
        <b className="font-semibold">{eventTitle(t, event.title)}</b>
      </p>
      <AskToggle event={event} />
    </section>
  );
}
