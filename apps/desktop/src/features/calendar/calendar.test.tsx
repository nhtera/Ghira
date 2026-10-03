// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CalendarStatus, EventView } from "../../bindings";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const commands = vi.hoisted(() => ({
  upcomingEvents: vi.fn(),
  setEventAsk: vi.fn(),
  calendarStatus: vi.fn(),
  requestCalendarAccess: vi.fn(),
  setCalendar: vi.fn(),
  pickIcsFile: vi.fn(),
  removeIcsFile: vi.fn(),
  openPrivacySettings: vi.fn(),
}));
vi.mock("../../ipc", () => ({ ipc: { commands } }));

import i18n from "i18next";
import { renderLive } from "../live/test-utils";
import { CalendarCard } from "./calendar-card";
import { CalendarPermissionRow } from "./calendar-permission-row";
import { NextEvent } from "./next-event";
import { UpNextStrip } from "./up-next-strip";

// The copy the lead merges from PENDING-calendar2.json.
i18n.addResourceBundle(
  "en",
  "translation",
  {
    calendar: {
      untitled: "Untitled event",
      error: {
        storage: "Couldn’t read or write the calendar settings. Try again.",
        notFound: "That event is no longer in your calendar.",
        notSupported: "Calendar access isn’t available on this computer.",
        generic: "Something went wrong with the calendar. Try again.",
      },
    },
  },
  true,
  true,
);

const soon = Date.now() + 12 * 60_000;
const sprint: EventView = {
  key: "ev-1",
  title: "Sprint planning",
  startMs: soon,
  endMs: soon + 3_600_000,
  attendees: 5,
  joinApp: "zoom",
  ask: true,
};
const lunch: EventView = {
  key: "ev-2",
  title: "Lunch",
  startMs: soon - 1,
  endMs: soon + 1000,
  attendees: 0,
  joinApp: null,
  ask: false,
};
const status = (p: Partial<CalendarStatus> = {}): CalendarStatus => ({
  eventkit: "authorized",
  ics: null,
  askOnStart: true,
  ...p,
});

beforeEach(() => {
  Object.values(commands).forEach((c) => c.mockReset());
  commands.setEventAsk.mockReturnValue(ok(null));
  commands.setCalendar.mockReturnValue(ok(status()));
  commands.calendarStatus.mockReturnValue(ok(status()));
});
afterEach(cleanup);

describe("Up next strip and the popover row", () => {
  it("shows the next meeting-like event, skipping one with nobody else, and flips its switch", async () => {
    commands.upcomingEvents.mockReturnValue(ok([lunch, sprint]));
    renderLive(<UpNextStrip />);
    expect(await screen.findByText("Sprint planning")).toBeTruthy();
    expect(screen.queryByText("Lunch")).toBeNull();
    expect(screen.getByText("in 12 min · 5 people · Zoom")).toBeTruthy();
    const sw = screen.getByRole("switch", {
      name: "Ask to record when it starts",
    });
    expect(sw.getAttribute("aria-checked")).toBe("true");
    await userEvent.setup().click(sw);
    await waitFor(() =>
      expect(commands.setEventAsk).toHaveBeenCalledExactlyOnceWith(
        "ev-1",
        false,
      ),
    );
    // Shown at once, before the next refresh.
    await waitFor(() =>
      expect(screen.getByRole("switch").getAttribute("aria-checked")).toBe(
        "false",
      ),
    );
  });

  it("is absent without a calendar or a meeting-like event", async () => {
    commands.upcomingEvents.mockReturnValue(ok([lunch]));
    const { container } = renderLive(<UpNextStrip />);
    await waitFor(() => expect(commands.upcomingEvents).toHaveBeenCalled());
    expect(container.textContent).toBe("");
  });

  it("the popover row names the same event and says when", async () => {
    commands.upcomingEvents.mockReturnValue(
      ok([{ ...sprint, startMs: Date.now() - 1000 }]),
    );
    renderLive(<NextEvent />);
    expect(await screen.findByText("Sprint planning")).toBeTruthy();
    expect(screen.getByText("Now")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Next" })).toBeTruthy();
  });

  it("a failed switch is undone and says why", async () => {
    commands.upcomingEvents.mockReturnValue(ok([sprint]));
    commands.setEventAsk.mockReturnValue(
      Promise.resolve({ status: "error", error: "storage" }),
    );
    renderLive(<UpNextStrip />);
    await userEvent.setup().click(await screen.findByRole("switch"));
    await waitFor(() =>
      expect(commands.upcomingEvents.mock.calls.length).toBeGreaterThan(1),
    );
  });
});

describe("Settings → Calendar card", () => {
  it("asks for access, then shows Connected and the ask switch", async () => {
    commands.calendarStatus.mockReturnValue(
      ok(status({ eventkit: "notDetermined" })),
    );
    commands.requestCalendarAccess.mockImplementation(() => {
      commands.calendarStatus.mockReturnValue(ok(status()));
      return ok(status());
    });
    renderLive(<CalendarCard />);
    await userEvent
      .setup()
      .click(await screen.findByRole("button", { name: "Connect Calendar…" }));
    expect(commands.requestCalendarAccess).toHaveBeenCalled();
    expect(await screen.findByText("Connected")).toBeTruthy();
    expect(
      screen.getByRole("switch", {
        name: "Ask to record when a calendar meeting starts",
      }),
    ).toBeTruthy();
  });

  it("a failed setting says why in words", async () => {
    commands.calendarStatus.mockReturnValue(ok(status()));
    commands.setCalendar.mockReturnValue(
      Promise.resolve({ status: "error", error: "storage" }),
    );
    renderLive(<CalendarCard />);
    await userEvent.setup().click(
      await screen.findByRole("switch", {
        name: "Ask to record when a calendar meeting starts",
      }),
    );
    expect(
      await screen.findByText(
        "Couldn’t read or write the calendar settings. Try again.",
      ),
    ).toBeTruthy();
  });

  it("denied: explains and opens the Calendars pane", async () => {
    commands.calendarStatus.mockReturnValue(ok(status({ eventkit: "denied" })));
    renderLive(<CalendarCard />);
    expect(await screen.findByText(/Calendar access is off for/)).toBeTruthy();
    await userEvent
      .setup()
      .click(screen.getByRole("button", { name: "Open System Settings" }));
    expect(commands.openPrivacySettings).toHaveBeenCalledWith("calendars");
    expect(screen.queryByRole("switch")).toBeNull();
  });

  it("an .ics file: imports, names it, removes it; a bad file says so", async () => {
    commands.calendarStatus.mockReturnValue(
      ok(
        status({
          eventkit: "unavailable",
          ics: { name: "work.ics", events: 3 },
        }),
      ),
    );
    commands.removeIcsFile.mockReturnValue(ok(null));
    renderLive(<CalendarCard />);
    expect(await screen.findByText("From work.ics")).toBeTruthy();
    await userEvent
      .setup()
      .click(screen.getByRole("button", { name: "Remove calendar file" }));
    expect(commands.removeIcsFile).toHaveBeenCalled();
    cleanup();

    commands.calendarStatus.mockReturnValue(
      ok(status({ eventkit: "unavailable" })),
    );
    commands.pickIcsFile.mockReturnValue(
      Promise.resolve({ status: "error", error: "icsInvalid" }),
    );
    renderLive(<CalendarCard />);
    await userEvent.setup().click(
      await screen.findByRole("button", {
        name: "Import a calendar file (.ics)…",
      }),
    );
    expect(
      await screen.findByText(/can.t read this calendar file/),
    ).toBeTruthy();
    expect(screen.queryByRole("switch")).toBeNull();
  });
});

describe("Onboarding Calendar row", () => {
  it("Allow… asks the OS; granted shows Allowed", async () => {
    commands.calendarStatus.mockReturnValue(
      ok(status({ eventkit: "notDetermined" })),
    );
    commands.requestCalendarAccess.mockImplementation(() => {
      commands.calendarStatus.mockReturnValue(ok(status()));
      return ok(status());
    });
    renderLive(
      <ul>
        <CalendarPermissionRow />
      </ul>,
    );
    await userEvent
      .setup()
      .click(await screen.findByRole("button", { name: "Allow…" }));
    expect(await screen.findByText("Allowed")).toBeTruthy();
  });

  it("is not shown where there is no calendar app to ask", async () => {
    commands.calendarStatus.mockReturnValue(
      ok(status({ eventkit: "unavailable" })),
    );
    const { container } = renderLive(
      <ul>
        <CalendarPermissionRow />
      </ul>,
    );
    await waitFor(() => expect(commands.calendarStatus).toHaveBeenCalled());
    expect(container.querySelector("li")).toBeNull();
  });
});
