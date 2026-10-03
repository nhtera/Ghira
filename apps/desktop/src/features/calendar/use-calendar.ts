// SPDX-License-Identifier: Apache-2.0
// Shared calendar queries and the small rules the Up next strip, the popover
// row and the settings card agree on (phase 14d).
import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { formatTime, type Locale } from "@ghi/i18n";
import { useToast } from "@ghi/ui";
import type { CalendarStatus, EventView } from "../../bindings";
import { ipc } from "../../ipc";

const UPCOMING = ["calendar", "upcoming"] as const;
export const calendarStatusKey = ["calendar", "status"] as const;

const ERROR_CODES = ["storage", "notFound", "notSupported"] as const;

/** A core error as a sentence: known codes get their own, anything else the generic line (never the code). */
export function calendarError(t: TFunction, code: string): string {
  const key = (ERROR_CODES as readonly string[]).includes(code)
    ? code
    : "generic";
  return (t as unknown as (key: string) => string)(`calendar.error.${key}`);
}

/** A title for an event that has none. */
export function eventTitle(t: TFunction, title: string): string {
  return (
    title.trim() ||
    (t as unknown as (key: string) => string)("calendar.untitled")
  );
}

/** Product names, not translated. */
const JOIN_APP: Record<string, string> = {
  zoom: "Zoom",
  teams: "Teams",
  meet: "Meet",
};
export const joinAppName = (app: string | null): string | null =>
  app ? (JOIN_APP[app] ?? null) : null;

/** Worth asking about: somebody else is in it, or it has a call link. */
export const isMeetingLike = (e: EventView): boolean =>
  e.attendees > 0 || e.joinApp != null;

/** The next events, refreshed every 30 s (the core caches for a minute). */
export function useUpcoming(limit: number) {
  return useQuery({
    queryKey: [...UPCOMING, limit],
    refetchInterval: 30_000,
    queryFn: async (): Promise<EventView[]> => {
      const r = await ipc.commands.upcomingEvents(limit);
      return r.status === "ok" ? r.data : [];
    },
  });
}

export function useCalendarStatus() {
  return useQuery({
    queryKey: calendarStatusKey,
    queryFn: async (): Promise<CalendarStatus | null> => {
      const r = await ipc.commands.calendarStatus();
      return r.status === "ok" ? r.data : null;
    },
  });
}

/** The people in the invite a recorded meeting was named after. */
export function useMeetingAttendees(
  meeting: string | null | undefined,
): string[] {
  const { data } = useQuery({
    queryKey: ["calendar", "attendees", meeting],
    enabled: Boolean(meeting),
    staleTime: 5 * 60_000,
    queryFn: async (): Promise<string[]> => {
      const r = await ipc.commands.meetingAttendees(meeting ?? "");
      return r.status === "ok" ? r.data : [];
    },
  });
  return data ?? [];
}

/** Turns "ask to record" on or off for one event, shown at once and undone on failure. */
export function useSetEventAsk() {
  const { t } = useTranslation();
  const { show } = useToast();
  const client = useQueryClient();
  return useCallback(
    async (key: string, ask: boolean) => {
      // A refresh in flight must not land over the change.
      await client.cancelQueries({ queryKey: UPCOMING });
      client.setQueriesData<EventView[]>({ queryKey: UPCOMING }, (rows) =>
        rows?.map((e) => (e.key === key ? { ...e, ask } : e)),
      );
      const r = await ipc.commands.setEventAsk(key, ask);
      if (r.status === "error") {
        void client.invalidateQueries({ queryKey: UPCOMING });
        show({
          tone: "warning",
          title: t("system.commandFailed", { message: r.error }),
        });
      }
    },
    [client, show, t],
  );
}

/** "Now", "in 12 min" or "at 2:30 PM". */
export function whenLabel(
  e: EventView,
  now: number,
  t: TFunction,
  locale: Locale,
): string {
  const start = e.startMs ?? now;
  if (start <= now) return t("calendar.now");
  const minutes = Math.ceil((start - now) / 60_000);
  return minutes < 60
    ? t("calendar.inMinutes", { count: minutes })
    : t("calendar.at", { time: formatTime(start, locale) });
}
