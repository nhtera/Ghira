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
  return (
    <section aria-label={t("tray.next")} className="flex flex-col gap-1.5">
      <h2 className="text-small m-0 px-1.5 font-semibold text-muted">
        {t("tray.next")}
      </h2>
      <div className="flex flex-col gap-1.5 rounded-seg bg-accent-soft px-2.5 py-2">
        <p className="m-0 flex min-w-0 items-center gap-2 text-[13px]">
          <Icon
            name="event_upcoming"
            size={16}
            className="flex-none text-accent"
          />
          <b className="min-w-0 flex-1 truncate font-semibold">
            {eventTitle(t, event.title)}
          </b>
          <span className="text-small flex-none text-muted">
            {whenLabel(event, now, t, locale)}
          </span>
        </p>
        <AskToggle event={event} />
      </div>
    </section>
  );
}
