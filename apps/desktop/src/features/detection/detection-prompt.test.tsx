// SPDX-License-Identifier: Apache-2.0
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MeetingDetected } from "../../bindings";

const h = vi.hoisted(() => ({
  emit: undefined as undefined | ((e: MeetingDetected) => void),
  reply: vi.fn(async () => ({ status: "ok", data: null })),
  start: vi.fn(async () => ({ status: "ok", data: "m1" })),
}));

vi.mock("../../ipc", () => ({
  ipc: {
    kind: "mock",
    commands: { replyMeetingDetected: h.reply },
    onMeetingDetected: async (cb: (e: MeetingDetected) => void) => {
      h.emit = cb;
      return () => {};
    },
  },
}));
vi.mock("../../shell/actions", () => ({
  useAppActions: () => ({ startRecording: h.start }),
}));
vi.mock("@ghi/ui", async (orig) => ({
  ...(await orig<typeof import("@ghi/ui")>()),
  useToast: () => ({ show: vi.fn() }),
}));

import { DetectionPrompt } from "./detection-prompt";

const zoom: MeetingDetected = { app: "zoom", appName: "Zoom", browser: false };
const detect = async () => {
  render(<DetectionPrompt />);
  await waitFor(() => expect(h.emit).toBeTruthy());
  fireEvent.click(document.body); // keep focus where it was
  h.emit!(zoom);
};

beforeEach(() => vi.clearAllMocks());
afterEach(cleanup);

describe("DetectionPrompt", () => {
  it("shows nothing until a meeting app is detected, and never takes focus", async () => {
    await detect();
    expect(
      await screen.findByRole("region", {
        name: "Zoom call detected. Record it?",
      }),
    ).toBeTruthy();
    expect(screen.getByRole("status").textContent).toBe(
      "Zoom call detected. Record it?",
    );
    expect(document.activeElement).toBe(document.body);
  });
  it("Start records a call and replies start", async () => {
    await detect();
    fireEvent.click(await screen.findByRole("button", { name: "Start" }));
    await waitFor(() => expect(h.reply).toHaveBeenCalledWith("zoom", "start"));
    expect(h.start).toHaveBeenCalledWith("call");
    expect(screen.queryByRole("region")).toBeNull();
  });
  it("Not now and Never reply and dismiss", async () => {
    await detect();
    fireEvent.click(await screen.findByRole("button", { name: "Not now" }));
    await waitFor(() => expect(h.reply).toHaveBeenCalledWith("zoom", "notNow"));
    h.emit!(zoom);
    fireEvent.click(
      await screen.findByRole("button", { name: "Never for Zoom" }),
    );
    await waitFor(() => expect(h.reply).toHaveBeenCalledWith("zoom", "never"));
    expect(h.start).not.toHaveBeenCalled();
  });
});
