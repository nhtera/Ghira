// SPDX-License-Identifier: Apache-2.0
// The calendar's connection to the core: Settings → Calendar (status, connect,
// turn off) and the event in progress the record screen shows.
import { useCallback, useEffect, useState } from "react";
import type { CalendarEvent, CalendarStatus } from "../../bindings";
import { ipc } from "../../ipc";
import { unwrap, useAction, useResource } from "../settings/api";

const loadStatus = async () => unwrap(await ipc.commands.calendarStatus());

/** Calendar access and the setting; `connect` asks iOS the first time (the answer comes back in the status). */
export function useCalendar() {
  const r = useResource<CalendarStatus>(loadStatus);
  const action = useAction();
  const { set } = r;
  const connect = useCallback(
    () =>
      action.run(async () => {
        set(unwrap(await ipc.commands.calendarConnect()));
        return true;
      }),
    [action, set],
  );
  const disconnect = useCallback(
    () =>
      action.run(async () => {
        set(unwrap(await ipc.commands.calendarDisconnect()));
        return true;
      }),
    [action, set],
  );
  return { status: r.data, loadError: r.error, reload: r.reload, connect, disconnect, busy: action.busy, error: action.error };
}

/** Events change under us (a meeting starts): look again this often while the screen is waiting. */
const REFRESH_MS = 60_000;

/**
 * The event in progress (or about to start) when the calendar is on, else
 * null. Looks again every minute and when the app comes back; a failure
 * (locked, not ready) is no event.
 */
export function useCurrentEvent(enabled: boolean): CalendarEvent | null {
  const [event, setEvent] = useState<CalendarEvent | null>(null);
  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    const look = () =>
      void ipc.commands.calendarCurrentEvent().then(
        (r) => alive && setEvent(r.status === "ok" ? r.data.event : null),
        () => alive && setEvent(null),
      );
    look();
    const timer = setInterval(look, REFRESH_MS);
    const onVisible = () => document.visibilityState === "visible" && look();
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      alive = false;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [enabled]);
  return enabled ? event : null;
}
