// SPDX-License-Identifier: Apache-2.0
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider } from "@ghi/ui";

const ok = { status: "ok", data: null };
const h = vi.hoisted(() => ({
  commands: {
    stopRecording: vi.fn(),
    pauseRecording: vi.fn(),
    resumeRecording: vi.fn(),
    markMoment: vi.fn(),
    showMain: vi.fn(),
    closeMini: vi.fn(),
    setMiniCompact: vi.fn(),
  },
}));
vi.mock("../../ipc", () => ({ ipc: { kind: "mock", commands: h.commands } }));

import { initialLive, useLive } from "../../state/live";
import { MiniRecorder } from "./mini-recorder";

const recording = {
  ...initialLive,
  meeting: "m",
  state: "recording" as const,
  startedAtMs: Date.now(),
  partial: { 0: "so the plan" },
};
const mount = () =>
  render(
    <PlatformProvider value="mac">
      <MiniRecorder />
    </PlatformProvider>,
  );

beforeEach(() => {
  vi.clearAllMocks();
  for (const k of Object.keys(h.commands) as (keyof typeof h.commands)[])
    h.commands[k].mockResolvedValue(ok);
  useLive.setState(recording);
});
afterEach(cleanup);

describe("MiniRecorder", () => {
  it("shows the latest words and controls the recording", () => {
    mount();
    expect(screen.getByText("so the plan")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Mark moment" }));
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(h.commands.markMoment).toHaveBeenCalled();
    expect(h.commands.pauseRecording).toHaveBeenCalled();
    expect(h.commands.stopRecording).toHaveBeenCalled();
  });

  it("offers Resume while paused", () => {
    useLive.setState({ ...recording, state: "paused", pausedAtMs: Date.now() });
    mount();
    fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    expect(h.commands.resumeRecording).toHaveBeenCalled();
  });

  it("collapses to a pill and expands again", async () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: "Shrink to a pill" }));
    await waitFor(() =>
      expect(h.commands.setMiniCompact).toHaveBeenCalledWith(true),
    );
    expect(await screen.findByRole("button", { name: "Expand" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Expand" }));
    await waitFor(() =>
      expect(h.commands.setMiniCompact).toHaveBeenCalledWith(false),
    );
    expect(await screen.findByRole("button", { name: "Stop" })).toBeTruthy();
  });

  it("opens Live in the main window and closes itself", async () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    await waitFor(() => expect(h.commands.closeMini).toHaveBeenCalled());
    expect(h.commands.showMain).toHaveBeenCalledWith("/live");
  });

  it("closes when the session ends", () => {
    mount();
    expect(h.commands.closeMini).not.toHaveBeenCalled();
    act(() => useLive.setState({ ...recording, state: "processing" }));
    expect(h.commands.closeMini).toHaveBeenCalled();
  });

  it("shows who is speaking in the full window", () => {
    useLive.setState({
      ...recording,
      partial: {},
      speakers: {
        2: {
          id: 2,
          label: "Speaker 2",
          colorSlot: 2,
          isMe: false,
          provisional: false,
          notPerson: false,
          others: false,
        },
      },
      lines: [
        {
          gid: "l",
          speaker: 2,
          t0Ms: 0,
          t1Ms: 1000,
          text: "hello there",
          overlap: false,
          words: [],
        },
      ],
    });
    mount();
    expect(screen.getByText("Speaker 2")).toBeTruthy();
    expect(screen.getByText("hello there")).toBeTruthy();
  });

  it("opens as a pill with #/mini?compact=1 and shows no text", () => {
    window.location.hash = "#/mini?compact=1";
    mount();
    window.location.hash = "";
    expect(screen.getByRole("button", { name: "Expand" })).toBeTruthy();
    expect(screen.queryByText("so the plan")).toBeNull();
  });
});
