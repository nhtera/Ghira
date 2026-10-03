// SPDX-License-Identifier: Apache-2.0
// The mock core's calendar side (phase 14d). W0-B baseline: the calendar is
// off unless a URL flag turns it on; slice S5 extends it.
//   ?calendar=1            access granted, two upcoming events
//   ?calendar=denied       access denied
//   ?calendar=ics          an ICS file is connected instead
import type { CalendarStatus, EventView } from "../bindings";
import type { Commands } from "./ipc";

type Result<T> = { status: "ok"; data: T } | { status: "error"; error: string };
const ok = <T>(data: T): Promise<Result<T>> =>
  Promise.resolve({ status: "ok", data });
const fail = <T>(error: string): Promise<Result<T>> =>
  Promise.resolve({ status: "error", error });
const flag = () => new URLSearchParams(location.search).get("calendar");

type CalendarCommands = Pick<
  Commands,
  | "calendarStatus"
  | "requestCalendarAccess"
  | "setCalendar"
  | "pickIcsFile"
  | "removeIcsFile"
  | "upcomingEvents"
  | "setEventAsk"
  | "meetingAttendees"
>;

export function calendarCommands(): CalendarCommands {
  let askOnStart = true;
  let ics = flag() === "ics";
  let eventkit =
    flag() === "1"
      ? "authorized"
      : flag() === "denied"
        ? "denied"
        : "notDetermined";
  const asks = new Map<string, boolean>();
  const events = (): EventView[] =>
    eventkit === "authorized" || ics
      ? [
          {
            key: "ev-1",
            title: "Sprint planning",
            startMs: Date.now() + 12 * 60_000,
            endMs: Date.now() + 72 * 60_000,
            attendees: 5,
            joinApp: "zoom",
            ask: asks.get("ev-1") ?? true,
          },
          {
            key: "ev-2",
            title: "Lunch",
            startMs: Date.now() + 3 * 3_600_000,
            endMs: Date.now() + 4 * 3_600_000,
            attendees: 0,
            joinApp: null,
            ask: asks.get("ev-2") ?? false,
          },
        ]
      : [];
  const status = (): CalendarStatus => ({
    eventkit,
    ics: ics ? { name: "work.ics", events: 2 } : null,
    askOnStart,
  });
  return {
    calendarStatus: () => ok(status()),
    requestCalendarAccess: () => {
      if (flag() === "denied") return ok(status());
      eventkit = "authorized";
      return ok(status());
    },
    setCalendar: (patch) => {
      if (patch.askOnStart != null) askOnStart = patch.askOnStart;
      return ok(status());
    },
    pickIcsFile: () => {
      ics = true;
      return ok("work.ics");
    },
    removeIcsFile: () => {
      ics = false;
      return ok(null);
    },
    upcomingEvents: (limit) => ok(events().slice(0, limit)),
    setEventAsk: (key, ask) => {
      if (!events().some((e) => e.key === key)) return fail("notFound");
      asks.set(key, ask);
      return ok(null);
    },
    // Only with the calendar on: other tests see no invite.
    meetingAttendees: () =>
      ok(flag() === "1" || flag() === "ics" ? ["Linh", "Minh", "Sarah"] : []),
  };
}
