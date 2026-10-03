// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { MeetingDetected } from "../../bindings";
import i18n from "i18next";
import { useLock } from "../../state/lock";
import { DetectionCard } from "./detection-prompt";
import { detectedFromHash } from "./detect-panel";

i18n.addResourceBundle(
  "en",
  "translation",
  {
    tray: {
      calendarStart: { untitled: "A calendar meeting is starting. Record it?" },
    },
  },
  true,
  true,
);

afterEach(() => {
  cleanup();
  useLock.setState({ locked: null });
});

describe("Detection card with a calendar event", () => {
  it("an app call inside an event carries its title as a subtitle", () => {
    const d: MeetingDetected = {
      app: "zoom",
      appName: "Zoom",
      browser: false,
      title: "Sprint planning",
      event: "ev-1@1",
    };
    render(<DetectionCard detected={d} onReply={vi.fn()} />);
    expect(screen.getByText("Zoom call detected. Record it?")).toBeTruthy();
    expect(
      screen.getByText("Sprint planning · from your calendar"),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Never for Zoom" })).toBeTruthy();
  });

  it("a calendar start asks about the event and offers no Never", () => {
    const onReply = vi.fn();
    const d: MeetingDetected = {
      app: "calendar",
      appName: "",
      browser: false,
      title: "Sprint planning",
      event: "ev-1@1",
    };
    render(<DetectionCard detected={d} onReply={onReply} />);
    expect(
      screen.getByText("“Sprint planning” is starting. Record it?"),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Never/ })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Not now" }));
    expect(onReply).toHaveBeenCalledWith("notNow");
    fireEvent.click(screen.getByRole("button", { name: "Start" }));
    expect(onReply).toHaveBeenCalledWith("start");
  });

  it("an event without a title does not show empty quotes", () => {
    const d: MeetingDetected = {
      app: "calendar",
      appName: "",
      browser: false,
      title: "  ",
      event: "e@1",
    };
    render(<DetectionCard detected={d} onReply={vi.fn()} />);
    expect(
      screen.getByText("A calendar meeting is starting. Record it?"),
    ).toBeTruthy();
    expect(document.body.textContent).not.toContain("“”");
  });

  it("while locked the title is not shown", () => {
    useLock.setState({ locked: true });
    const cal: MeetingDetected = {
      app: "calendar",
      appName: "",
      browser: false,
      title: "Layoffs plan",
      event: "e@1",
    };
    const { unmount } = render(
      <DetectionCard detected={cal} onReply={vi.fn()} />,
    );
    expect(document.body.textContent).not.toContain("Layoffs plan");
    expect(
      screen.getByText("A calendar meeting is starting. Record it?"),
    ).toBeTruthy();
    unmount();
    render(
      <DetectionCard
        detected={{ ...cal, app: "zoom", appName: "Zoom" }}
        onReply={vi.fn()}
      />,
    );
    expect(document.body.textContent).not.toContain("Layoffs plan");
  });

  it("the panel route carries the title and the event", () => {
    expect(
      detectedFromHash(
        "#/detect?app=calendar&name=&browser=0&title=Sprint%20planning&event=ev-1%401",
      ),
    ).toMatchObject({
      app: "calendar",
      title: "Sprint planning",
      event: "ev-1@1",
    });
    expect(
      detectedFromHash("#/detect?app=zoom&name=Zoom&browser=0")?.title,
    ).toBeUndefined();
  });
});

describe("Detect panel and the app lock", () => {
  it("locking closes the panel and takes the title out of its address", async () => {
    const closeDetect = vi.fn();
    vi.doMock("../../ipc", () => ({
      ipc: {
        commands: {
          closeDetect,
          startRecording: vi.fn(),
          replyMeetingDetected: vi.fn(),
          openMiniRecorder: vi.fn(),
        },
        onMeetingDetected: async () => () => {},
      },
    }));
    vi.doMock("../popover/panel-window", () => ({ usePanelWindow: () => {} }));
    vi.resetModules();
    const { DetectPanel } = await import("./detect-panel");
    const { act } = await import("@testing-library/react");
    window.location.hash =
      "#/detect?app=calendar&name=&browser=0&title=Layoffs%20plan&event=e%401";
    render(<DetectPanel />);
    expect(document.body.textContent).toContain("Layoffs plan");
    const { useLock: lock } = await import("../../state/lock");
    act(() => lock.setState({ locked: true }));
    expect(document.body.textContent).not.toContain("Layoffs plan");
    expect(window.location.hash).toBe("#/detect");
    expect(closeDetect).toHaveBeenCalled();
    vi.doUnmock("../../ipc");
    vi.doUnmock("../popover/panel-window");
  });
});
