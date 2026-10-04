// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, renderHook, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { initMobileI18n } from "@ghi/i18n/mobile";
import { PlatformProvider } from "@ghi/ui";
import { ipc } from "../../ipc";
import { CalendarCard, useCurrentEvent } from ".";

const cal = () => window.__ghiCalendar!;

beforeEach(() => cal().reset());
afterEach(cleanup);

describe("the scripted calendar", () => {
  it("starts off and unasked, so no other screen shows it", async () => {
    const s = await ipc.commands.calendarStatus();
    expect(s).toEqual({ status: "ok", data: { access: "notDetermined", connected: false } });
    expect(await ipc.commands.calendarCurrentEvent()).toEqual({ status: "ok", data: { event: null } });
  });

  it("connects when the prompt is allowed, and turning it off hides the event again", async () => {
    const on = await ipc.commands.calendarConnect();
    expect(on).toEqual({ status: "ok", data: { access: "authorized", connected: true } });
    const now = await ipc.commands.calendarCurrentEvent();
    expect(now.status === "ok" && now.data.event?.title).toBe("Weekly sync");
    const off = await ipc.commands.calendarDisconnect();
    expect(off).toEqual({ status: "ok", data: { access: "authorized", connected: false } });
    expect(await ipc.commands.calendarCurrentEvent()).toEqual({ status: "ok", data: { event: null } });
  });

  it("stays off when the prompt is refused", async () => {
    cal().promptAnswer = "denied";
    const r = await ipc.commands.calendarConnect();
    expect(r).toEqual({ status: "ok", data: { access: "denied", connected: false } });
  });
});

describe("the record screen's calendar note", () => {
  const wrap = (ui: React.ReactNode) =>
    render(
      <I18nextProvider i18n={initMobileI18n("en")}>
        <PlatformProvider value="ios">{ui}</PlatformProvider>
      </I18nextProvider>,
    );

  it("shows the event's title as plain text next to what it does", () => {
    wrap(<CalendarCard event={{ title: "<b>Q4</b> review", startMs: 0, endMs: 1, attendees: 2, joinApp: null }} />);
    const card = screen.getByTestId("calendar-card");
    expect(card.textContent).toContain("<b>Q4</b> review");
    expect(card.querySelector("b")).toBeNull();
    expect(card.textContent).toContain("From your calendar");
    expect(card.textContent).toContain("This recording will be named after it.");
  });

  it("asks the core only while idle and connected", async () => {
    const off = renderHook(() => useCurrentEvent(false));
    expect(off.result.current).toBeNull();
    expect(cal().calls.calendarCurrentEvent ?? 0).toBe(0);
    off.unmount();

    await ipc.commands.calendarConnect();
    const on = renderHook(() => useCurrentEvent(true));
    await waitFor(() => expect(on.result.current?.title).toBe("Weekly sync"));
    on.unmount();

    await ipc.commands.calendarDisconnect();
    const calls = cal().calls.calendarCurrentEvent ?? 0;
    const after = renderHook(() => useCurrentEvent(true));
    await waitFor(() => expect(cal().calls.calendarCurrentEvent).toBe(calls + 1));
    expect(after.result.current).toBeNull();
  });
});
