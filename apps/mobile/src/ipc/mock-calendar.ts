// SPDX-License-Identifier: Apache-2.0
// Scripted calendar commands: iOS access (the system prompt answers what the
// test says), the connected switch and the event in progress. Tests drive it
// through `window.__ghiCalendar`; it starts off, not asked yet, so no other
// screen shows a calendar.
import type { CalendarAccess, CalendarEvent, CalendarStatus } from "../bindings";
import type { Commands } from "./ipc";

const ok = <T>(data: T) => ({ status: "ok" as const, data });

export interface GhiCalendarMock {
  /** What iOS says now. */
  access: CalendarAccess;
  /** The setting (only counts while access is authorized). */
  connected: boolean;
  /** What the system prompt answers when the app asks. */
  promptAnswer: "authorized" | "denied";
  /** The event in progress (shown once connected). */
  event: CalendarEvent | null;
  /** How many times each command was called. */
  calls: Record<string, number>;
  /** Back to a phone that was never asked. */
  reset(): void;
}

declare global {
  interface Window {
    __ghiCalendar?: GhiCalendarMock;
  }
}

const sync = (): CalendarEvent => ({
  title: "Weekly sync",
  startMs: Date.now() - 5 * 60_000,
  endMs: Date.now() + 25 * 60_000,
  attendees: 2,
  joinApp: "zoom",
});

const hooks: GhiCalendarMock = {
  access: "notDetermined",
  connected: false,
  promptAnswer: "authorized",
  event: sync(),
  calls: {},
  reset() {
    Object.assign(hooks, { access: "notDetermined", connected: false, promptAnswer: "authorized", event: sync(), calls: {} });
  },
};
if (typeof window !== "undefined") window.__ghiCalendar = hooks;

const status = (): CalendarStatus => ({ access: hooks.access, connected: hooks.connected && hooks.access === "authorized" });
const count = (name: string) => (hooks.calls[name] = (hooks.calls[name] ?? 0) + 1);

export const calendarCommands: Partial<Commands> = {
  calendarStatus: async () => (count("calendarStatus"), ok(status())),
  calendarConnect: async () => {
    count("calendarConnect");
    if (hooks.access === "notDetermined") hooks.access = hooks.promptAnswer;
    hooks.connected = hooks.access === "authorized";
    return ok(status());
  },
  calendarDisconnect: async () => {
    count("calendarDisconnect");
    hooks.connected = false;
    return ok(status());
  },
  calendarCurrentEvent: async () => (count("calendarCurrentEvent"), ok({ event: status().connected ? hooks.event : null })),
  meetingAttendees: async () => ok([]),
  meetingContacts: async () => ok([]),
};
