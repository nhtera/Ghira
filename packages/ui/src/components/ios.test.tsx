// SPDX-License-Identifier: Apache-2.0
import { initMobileI18n } from "@ghi/i18n/mobile";
import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { PlatformProvider } from "../platform/platform";
import { Banner } from "./banner";
import { ListRow, ListSection } from "./list";
import { LargeTitle, NavBar, useLargeTitleCollapse } from "./nav-bar";
import { PhoneButton } from "./phone-button";
import { PrivacyIndicator } from "./privacy-indicator";
import { RecordControl } from "./record-control";
import { StatusPill, SyncChip, type SyncChipKind } from "./status-pill";
import { TabBar } from "./tab-bar";

beforeAll(() => {
  initMobileI18n("en");
});
afterEach(cleanup);

const ios = (ui: React.ReactElement) => render(<PlatformProvider value="ios">{ui}</PlatformProvider>);

describe("TabBar", () => {
  const items = [
    { id: "meetings", label: "Meetings", icon: "format_list_bulleted" as const },
    { id: "record", label: "Record", icon: "mic" as const, emphasized: true },
    { id: "search", label: "Search", icon: "search" as const },
    { id: "settings", label: "Settings", icon: "settings" as const },
  ];

  it("marks the selected tab and reports taps", async () => {
    const onChange = vi.fn();
    render(<TabBar label="Tabs" items={items} value="record" onChange={onChange} />);
    expect(screen.getByRole("navigation", { name: "Tabs" })).toBeTruthy();
    expect(screen.getAllByRole("button")).toHaveLength(4);
    expect(screen.getByRole("button", { name: "Record" }).getAttribute("aria-current")).toBe("page");
    expect(screen.getByRole("button", { name: "Search" }).getAttribute("aria-current")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Search" }));
    expect(onChange).toHaveBeenCalledWith("search");
  });
});

describe("NavBar", () => {
  it("keeps one h1, the large title; the inline copy is hidden from assistive tech", () => {
    render(
      <>
        <NavBar title="Meetings" />
        <LargeTitle>Meetings</LargeTitle>
      </>,
    );
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
    expect(screen.getByRole("heading", { name: "Meetings" })).toBeTruthy();
  });

  it("a standard bar puts the title in the bar", () => {
    render(<NavBar title="Notes" large={false} />);
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
  });

  it("back button is named Back, with the previous screen when shown", async () => {
    const onBack = vi.fn();
    const { rerender } = render(<NavBar title="Notes" onBack={onBack} />);
    await userEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalledOnce();
    rerender(<NavBar title="Notes" onBack={onBack} backLabel="Meetings" />);
    expect(screen.getByRole("button", { name: "Back, Meetings" })).toBeTruthy();
  });

  describe("useLargeTitleCollapse", () => {
    const observers: { cb: IntersectionObserverCallback; root: unknown; disconnected: boolean }[] = [];
    beforeAll(() => {
      vi.stubGlobal(
        "IntersectionObserver",
        class {
          entry = { cb: null as unknown as IntersectionObserverCallback, root: null as unknown, disconnected: false };
          constructor(cb: IntersectionObserverCallback, opts?: IntersectionObserverInit) {
            this.entry = { cb, root: opts?.root, disconnected: false };
            observers.push(this.entry);
          }
          observe() {}
          disconnect() {
            this.entry.disconnected = true;
          }
        },
      );
    });
    afterAll(() => vi.unstubAllGlobals());

    function Page() {
      const { collapsed, scrollRef, titleRef } = useLargeTitleCollapse();
      return (
        <div>
          <NavBar title="Meetings" collapsed={collapsed} />
          <div ref={scrollRef}>
            <LargeTitle ref={titleRef}>Meetings</LargeTitle>
          </div>
        </div>
      );
    }

    it("collapses when the large title leaves the scroller, and back", () => {
      const { container, unmount } = render(<Page />);
      const o = observers.at(-1)!;
      expect(o.root).toBe(container.querySelector("h1")!.parentElement);
      expect(container.querySelector("header")?.getAttribute("data-collapsed")).toBe("false");
      act(() => o.cb([{ isIntersecting: false } as IntersectionObserverEntry], {} as IntersectionObserver));
      expect(container.querySelector("header")?.getAttribute("data-collapsed")).toBe("true");
      act(() => o.cb([{ isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver));
      expect(container.querySelector("header")?.getAttribute("data-collapsed")).toBe("false");
      unmount();
      expect(o.disconnected).toBe(true);
    });
  });
});

describe("List", () => {
  it("labels the group by its header; a pressable row is a button, others are not", async () => {
    const onPress = vi.fn();
    render(
      <ListSection header="Privacy" footer="Stays on this phone.">
        <ListRow title="App lock" chevron onPress={onPress} />
        <ListRow title="Version" value="1.0" />
      </ListSection>,
    );
    expect(screen.getByRole("list", { name: "Privacy" })).toBeTruthy();
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
    expect(screen.getAllByRole("button")).toHaveLength(1);
    await userEvent.click(screen.getByRole("button", { name: "App lock" }));
    expect(onPress).toHaveBeenCalledOnce();
    expect(screen.getByText("Stays on this phone.")).toBeTruthy();
  });

  it("a trailing control is named by the row title", () => {
    render(
      <ListSection>
        <ListRow title="App lock" trailing={(id) => <input type="checkbox" role="switch" aria-labelledby={id} />} />
      </ListSection>,
    );
    expect(screen.getByRole("switch", { name: "App lock" })).toBeTruthy();
  });

  it("a disabled row cannot be pressed", async () => {
    const onPress = vi.fn();
    render(
      <ListSection>
        <ListRow title="Locked" onPress={onPress} disabled />
      </ListSection>,
    );
    await userEvent.click(screen.getByRole("button"));
    expect(onPress).not.toHaveBeenCalled();
  });
});

describe("Banner", () => {
  it("shows title and detail, with an action and a dismiss", async () => {
    const onPress = vi.fn();
    const onDismiss = vi.fn();
    render(
      <Banner variant="warning" title="Too hot" action={{ label: "Learn more", onPress }} onDismiss={onDismiss} dismissLabel="Dismiss">
        Transcription pauses.
      </Banner>,
    );
    expect(screen.getByRole("status").getAttribute("data-variant")).toBe("warning");
    expect(screen.getByText("Transcription pauses.")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Learn more" }));
    await userEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(onPress).toHaveBeenCalledOnce();
    expect(onDismiss).toHaveBeenCalledOnce();
  });

  it("has an icon next to the text", () => {
    const { container } = render(<Banner variant="info" title="Record only" />);
    expect(container.querySelector("svg[data-icon='info']")).toBeTruthy();
  });
});

describe("SyncChip", () => {
  const CASES: [SyncChipKind, string][] = [
    [{ kind: "recorded" }, "Recorded"],
    [{ kind: "processingOnPhone", percent: 61.6 }, "Processing on phone · 62%"],
    [{ kind: "processedOnPhone" }, "Processed on phone"],
    [{ kind: "waitingForModels" }, "Waiting for models"],
    [{ kind: "failed" }, "Failed"],
    [{ kind: "synced" }, "Synced"],
    [{ kind: "waitingForWifi" }, "Waiting for Wi-Fi"],
    [{ kind: "waitingForComputer" }, "Waiting for MacBook"],
    [{ kind: "finalOnDesktop", percent: 40 }, "Final pass on MacBook · 40%"],
  ];
  it.each(CASES)("%j shows an icon and text", (chip, text) => {
    const { container } = render(<SyncChip chip={chip} device="MacBook" />);
    expect(screen.getByText(text)).toBeTruthy();
    expect(container.querySelector("svg[data-icon]")).toBeTruthy();
  });

  it("failed is a retry button only when onRetry is given", async () => {
    const onRetry = vi.fn();
    const { rerender } = render(<SyncChip chip={{ kind: "failed" }} />);
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.getByText("Failed")).toBeTruthy();
    rerender(<SyncChip chip={{ kind: "failed" }} onRetry={onRetry} />);
    await userEvent.click(screen.getByRole("button", { name: "Failed · Tap to retry" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it("finalOnDesktop falls back to a generic computer name", () => {
    render(<SyncChip chip={{ kind: "finalOnDesktop", percent: 5 }} />);
    expect(screen.getByText("Final pass on My computer · 5%")).toBeTruthy();
  });
});

describe("iOS variants of shared components", () => {
  it("StatusPill keeps its text and icon and wraps (no fixed height)", () => {
    const { container } = ios(<StatusPill status="processing" percent={62} />);
    expect(screen.getByText("Processing 62%")).toBeTruthy();
    expect(container.querySelector("[data-status]")?.className).toContain("min-h-");
  });

  it("PrivacyIndicator is compact on iOS and unchanged on desktop", () => {
    const { container, rerender } = ios(<PrivacyIndicator state="local" />);
    expect(container.querySelector("[data-state]")?.className).toContain("text-ios-caption2");
    rerender(<PlatformProvider value="mac"><PrivacyIndicator state="local" /></PlatformProvider>);
    expect(container.querySelector("[data-state]")?.className).toContain("text-[12px]");
  });

  it("RecordControl is the thumb variant on iOS: big stop, pause beside it, no mode menu", async () => {
    const onStop = vi.fn();
    const onPause = vi.fn();
    ios(<RecordControl state="recording" mode="room" elapsedMs={65_000} onStop={onStop} onPause={onPause} />);
    expect(screen.getByText("01:05")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Stop" }));
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(onStop).toHaveBeenCalledOnce();
    expect(onPause).toHaveBeenCalledOnce();
  });

  it("RecordControl on iOS: idle starts in the current mode, paused offers Resume, error offers Fix", async () => {
    const onStart = vi.fn();
    const onResume = vi.fn();
    const onFix = vi.fn();
    const { rerender } = ios(<RecordControl state="idle" mode="room" onStart={onStart} />);
    expect(screen.queryByRole("button", { name: /choose/i })).toBeNull();
    await userEvent.click(screen.getAllByRole("button")[0]);
    expect(onStart).toHaveBeenCalledWith("room");
    rerender(<PlatformProvider value="ios"><RecordControl state="paused" mode="room" onResume={onResume} /></PlatformProvider>);
    await userEvent.click(screen.getByRole("button", { name: "Resume" }));
    expect(onResume).toHaveBeenCalledOnce();
    rerender(<PlatformProvider value="ios"><RecordControl state="error" mode="room" onFix={onFix} /></PlatformProvider>);
    await userEvent.click(screen.getByRole("button", { name: "Fix" }));
    expect(onFix).toHaveBeenCalledOnce();
  });
});

describe("PhoneButton", () => {
  it("is a 56 pt button that fills the row, wraps its label and reports taps", async () => {
    const onClick = vi.fn();
    ios(<PhoneButton onClick={onClick}>Start recording a long meeting</PhoneButton>);
    const b = screen.getByRole("button", { name: "Start recording a long meeting" });
    expect(b.className).toContain("min-h-ios-button");
    expect(b.className).toContain("w-full");
    expect(b.getAttribute("type")).toBe("button");
    await userEvent.click(b);
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("hugs the label when inline and ignores taps while disabled", async () => {
    const onClick = vi.fn();
    ios(<PhoneButton inline disabled onClick={onClick}>Save</PhoneButton>);
    const b = screen.getByRole("button", { name: "Save" });
    expect(b.className).not.toContain("w-full");
    await userEvent.click(b);
    expect(onClick).not.toHaveBeenCalled();
  });
});

describe("ListRow disclosure", () => {
  it("exposes aria-expanded and aria-controls on a row that expands content", () => {
    ios(
      <ListSection>
        <ListRow title="Row" onPress={() => {}} expanded controls="detail" />
      </ListSection>,
    );
    const b = screen.getByRole("button", { name: "Row" });
    expect(b.getAttribute("aria-expanded")).toBe("true");
    expect(b.getAttribute("aria-controls")).toBe("detail");
  });
  it("sets neither on a plain row", () => {
    ios(
      <ListSection>
        <ListRow title="Plain" onPress={() => {}} chevron />
      </ListSection>,
    );
    const b = screen.getByRole("button", { name: "Plain" });
    expect(b.hasAttribute("aria-expanded")).toBe(false);
    expect(b.hasAttribute("aria-controls")).toBe(false);
  });
});
