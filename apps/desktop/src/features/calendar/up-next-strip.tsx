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
  const details = [
    whenLabel(event, now, t, locale),
    event.attendees > 0
      ? t("calendar.people", { count: event.attendees })
      : null,
    app,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <section
      aria-label={t("library.upNext")}
      className="mb-3 flex flex-none flex-wrap items-center gap-x-4 gap-y-2 rounded-panel bg-accent-soft px-3.5 py-2.5"
    >
      <Icon name="event_upcoming" size={20} className="flex-none text-accent" />
      <div className="min-w-0 flex-1">
        <p className="text-small m-0 font-semibold text-accent">
          {t("library.upNext")}
        </p>
        <p className="m-0 truncate text-[13.5px]">
          <b className="font-semibold">{eventTitle(t, event.title)}</b>
        </p>
        <p className="text-small m-0 text-muted">{details}</p>
      </div>
      <AskToggle event={event} />
    </section>
  );
}
