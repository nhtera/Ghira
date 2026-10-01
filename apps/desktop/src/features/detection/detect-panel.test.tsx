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

const ok = { status: "ok", data: null };
const h = vi.hoisted(() => ({
  onDetected: undefined as undefined | ((d: unknown) => void),
  commands: {
    startRecording: vi.fn(),
    replyMeetingDetected: vi.fn(),
    openMiniRecorder: vi.fn(),
    closeDetect: vi.fn(),
  },
}));
vi.mock("../../ipc", () => ({
  ipc: {
    kind: "mock",
    commands: h.commands,
    onMeetingDetected: async (cb: (d: unknown) => void) => {
      h.onDetected = cb;
      return () => {};
    },
  },
}));

import { DetectPanel, detectedFromHash } from "./detect-panel";

const zoom = { app: "zoom", appName: "Zoom", browser: false };

beforeEach(() => {
  vi.clearAllMocks();
  for (const k of Object.keys(h.commands) as (keyof typeof h.commands)[])
    h.commands[k].mockResolvedValue(ok);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("detectedFromHash", () => {
  it("reads the app from the route search", () => {
    expect(
      detectedFromHash("#/detect?app=chrome&name=Google%20Meet&browser=1"),
    ).toEqual({ app: "chrome", appName: "Google Meet", browser: true });
    expect(detectedFromHash("#/detect")).toBeNull();
  });
});

describe("DetectPanel", () => {
  it("Start records, replies, opens the mini recorder and closes", async () => {
    render(<DetectPanel initial={zoom} />);
    fireEvent.click(screen.getByRole("button", { name: "Start" }));
    await waitFor(() => expect(h.commands.closeDetect).toHaveBeenCalled());
    expect(h.commands.startRecording).toHaveBeenCalledWith("call", null, "");
    expect(h.commands.replyMeetingDetected).toHaveBeenCalledWith(
      "zoom",
      "start",
    );
    expect(h.commands.openMiniRecorder).toHaveBeenCalled();
  });

  it("a failed start stays open with the error", async () => {
    h.commands.startRecording.mockResolvedValue({
      status: "error",
      error: "mic denied",
    });
    render(<DetectPanel initial={zoom} />);
    fireEvent.click(screen.getByRole("button", { name: "Start" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "mic denied",
    );
    expect(h.commands.closeDetect).not.toHaveBeenCalled();
  });

  it("Never replies and closes", async () => {
    render(<DetectPanel initial={zoom} />);
    fireEvent.click(screen.getByRole("button", { name: "Never for Zoom" }));
    await waitFor(() => expect(h.commands.closeDetect).toHaveBeenCalled());
    expect(h.commands.replyMeetingDetected).toHaveBeenCalledWith(
      "zoom",
      "never",
    );
    expect(h.commands.startRecording).not.toHaveBeenCalled();
  });

  it("no answer for 30 s means Not now", async () => {
    vi.useFakeTimers();
    render(<DetectPanel initial={zoom} />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(h.commands.replyMeetingDetected).toHaveBeenCalledWith(
      "zoom",
      "notNow",
    );
    expect(h.commands.closeDetect).toHaveBeenCalled();
  });

  it("Escape means Not now", async () => {
    render(<DetectPanel initial={zoom} />);
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() =>
      expect(h.commands.replyMeetingDetected).toHaveBeenCalledWith(
        "zoom",
        "notNow",
      ),
    );
  });

  it("a later detection replaces the prompt and restarts the 30 s timer", async () => {
    vi.useFakeTimers();
    render(<DetectPanel initial={zoom} />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20_000);
    });
    act(() =>
      h.onDetected?.({ app: "teams", appName: "Teams", browser: false }),
    );
    expect(
      screen.getByRole("region", { name: "Teams call detected. Record it?" }),
    ).toBeTruthy();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20_000);
    });
    expect(h.commands.replyMeetingDetected).not.toHaveBeenCalled();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(h.commands.replyMeetingDetected).toHaveBeenCalledWith(
      "teams",
      "notNow",
    );
  });
});
