// SPDX-License-Identifier: Apache-2.0
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { i18next } from "@ghi/i18n";
import { PlatformProvider } from "@ghi/ui";

const ok = { status: "ok", data: null };
const h = vi.hoisted(() => ({
  commands: {
    listMeetings: vi.fn(),
    startRecording: vi.fn(),
    stopRecording: vi.fn(),
    pauseRecording: vi.fn(),
    resumeRecording: vi.fn(),
    showMain: vi.fn(),
    hidePopover: vi.fn(),
    requestQuitApp: vi.fn(),
    markMoment: vi.fn(),
  },
}));
vi.mock("../../ipc", () => ({ ipc: { kind: "mock", commands: h.commands } }));

import { initialLive, useLive } from "../../state/live";
import { Popover } from "./popover";

const row = (gid: string, title: string) => ({
  gid,
  title,
  startedAt: Date.now(),
  durationMs: 60_000,
  source: "call",
  mode: "call",
  status: "ready",
  transcriptVersion: 2,
  cloudUsed: false,
  consentConfirmed: false,
  job: null,
});

function mount() {
  return render(
    <QueryClientProvider client={new QueryClient()}>
      <PlatformProvider value="mac">
        <Popover />
      </PlatformProvider>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  i18next.addResourceBundle(
    "en",
    "translation",
    { tray: { quit: "Quit {{app}}" } },
    true,
  );
  useLive.setState(initialLive);
  h.commands.listMeetings.mockResolvedValue({
    status: "ok",
    data: [row("a", "Standup"), row("b", "")],
  });
  for (const k of [
    "startRecording",
    "stopRecording",
    "pauseRecording",
    "resumeRecording",
    "showMain",
    "hidePopover",
  ] as const)
    h.commands[k].mockResolvedValue(ok);
});
afterEach(cleanup);

describe("Popover", () => {
  it("starts a call in the main window and hides itself", async () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: "Record call" }));
    await waitFor(() => expect(h.commands.hidePopover).toHaveBeenCalled());
    expect(h.commands.startRecording).toHaveBeenCalledWith("call", null, "");
    expect(h.commands.showMain).toHaveBeenCalledWith("/live");
  });

  it("lists the last meetings and opens one", async () => {
    mount();
    fireEvent.click(await screen.findByRole("button", { name: /Standup/ }));
    await waitFor(() =>
      expect(h.commands.showMain).toHaveBeenCalledWith("/meetings/a/notes"),
    );
    expect(screen.getByText("Untitled meeting")).toBeTruthy();
  });

  it("shows the current recording with Stop, Pause and Open", async () => {
    useLive.setState({
      ...initialLive,
      meeting: "m",
      state: "recording",
      startedAtMs: Date.now(),
      session: {
        mode: "call",
        language: null,
        title: "Weekly",
        consentConfirmed: false,
      },
    });
    mount();
    expect(screen.getByText("Weekly")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Record call" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(h.commands.pauseRecording).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(h.commands.stopRecording).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    await waitFor(() =>
      expect(h.commands.showMain).toHaveBeenCalledWith("/live"),
    );
  });

  it("Escape hides the popover; Open Ghira shows the main window", async () => {
    mount();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(h.commands.hidePopover).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: /Open Ghira/ }));
    await waitFor(() => expect(h.commands.showMain).toHaveBeenCalledWith(null));
  });

  it("marks a moment and shows the speaker count while recording", () => {
    const sp = (id: number) => ({
      id,
      label: `Speaker ${id}`,
      colorSlot: id,
      isMe: false,
      provisional: false,
      notPerson: false,
      others: false,
    });
    useLive.setState({
      ...initialLive,
      meeting: "m",
      state: "recording",
      startedAtMs: Date.now(),
      speakers: { 1: sp(1), 2: sp(2) },
    });
    h.commands.markMoment.mockResolvedValue({ status: "ok", data: 1000 });
    mount();
    expect(screen.getByText("2 speakers")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Mark moment" }));
    expect(h.commands.markMoment).toHaveBeenCalled();
  });

  it("Quit asks the core to quit", () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: /Quit/ }));
    expect(h.commands.requestQuitApp).toHaveBeenCalled();
  });

  it("shows a failed start inline and stays open", async () => {
    h.commands.startRecording.mockResolvedValue({
      status: "error",
      error: "mic denied",
    });
    mount();
    fireEvent.click(screen.getByRole("button", { name: "Record call" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "mic denied",
    );
    expect(h.commands.hidePopover).not.toHaveBeenCalled();
  });
});
