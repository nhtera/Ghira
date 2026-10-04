// SPDX-License-Identifier: Apache-2.0
// The record screen's note about the calendar event in progress: the title
// the recording will get. Shown only while the calendar is connected and an
// event is on (the title is the user's own text: a text node, nothing else).
import { Icon } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import type { CalendarEvent } from "../../bindings";

export function CalendarCard({ event }: { event: CalendarEvent }) {
  const { t } = useTranslation();
  return (
    <div data-testid="calendar-card" className="flex shrink-0 items-start gap-3 rounded-(--ios-radius-group) bg-surface p-3">
      <Icon name="calendar_today" size={22} className="mt-0.5 size-[1.375rem] shrink-0 text-muted" />
      <p className="m-0 min-w-0">
        <span className="text-ios-footnote block text-muted">{t("mobile.record.calendar.label")}</span>
        <span className="text-ios-body block font-semibold break-words">{event.title}</span>
        <span className="text-ios-footnote block text-muted">{t("mobile.record.calendar.hint")}</span>
      </p>
    </div>
  );
}
