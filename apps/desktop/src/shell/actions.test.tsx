// SPDX-License-Identifier: Apache-2.0
import { renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { initialLive, useLive } from "../state/live";

const navigate = vi.fn();
const start = vi.hoisted(() => ({ resolve: (_: unknown) => {} }));
const commands = vi.hoisted(() => ({
  startRecording: vi.fn(() => new Promise((r) => (start.resolve = r))),
}));
vi.mock("../ipc", () => ({ ipc: { commands } }));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }));
vi.mock("@ghi/ui", () => ({ useToast: () => ({ show: vi.fn() }) }));

import { useAppActions } from "./actions";

afterEach(() => {
  useLive.setState({ ...initialLive });
  navigate.mockClear();
});

describe("startRecording", () => {
  it("shows the live screen, starting, before the core answers", async () => {
    const { result } = renderHook(() => useAppActions());
    const done = result.current.startRecording("room");
    expect(navigate).toHaveBeenCalledWith({ to: "/live" });
    expect(useLive.getState().state).toBe("starting");
    start.resolve({ status: "ok", data: "m1" });
    await done;
    expect(useLive.getState().state).toBe("starting"); // the core's events take it from here
  });

  it("puts the control back when the start fails", async () => {
    const { result } = renderHook(() => useAppActions());
    const done = result.current.startRecording("room");
    start.resolve({ status: "error", error: "microphone access is Denied" });
    await done;
    expect(useLive.getState().state).toBe("idle");
  });

  it("leaves a recording in progress alone", async () => {
    useLive.setState({ state: "recording", meeting: "m0" });
    const { result } = renderHook(() => useAppActions());
    const done = result.current.startRecording("room");
    start.resolve({ status: "error", error: "a recording is already running" });
    await done;
    expect(useLive.getState()).toMatchObject({ state: "recording", meeting: "m0" });
  });
});
