// SPDX-License-Identifier: Apache-2.0
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const ok = <T,>(data: T) => ({ status: "ok" as const, data });
const commands = vi.hoisted(() => ({
  voiceEnrollStart: vi.fn(),
  voiceEnrollStop: vi.fn(),
  voiceEnrollCancel: vi.fn(),
  enrollVoiceLevel: vi.fn(),
}));
vi.mock("../../ipc", () => ({ ipc: { commands } }));

import { MIN_SECONDS, useVoiceEnroll } from "./use-voice-enroll";

const level = (seconds: number, done = false) => ok({ level: 0.4, seconds, maxSeconds: 25, done });

beforeEach(() => {
  Object.values(commands).forEach((c) => c.mockReset());
  commands.voiceEnrollStart.mockResolvedValue(ok(null));
  commands.voiceEnrollStop.mockResolvedValue(ok(null));
  commands.voiceEnrollCancel.mockResolvedValue(ok(null));
});
afterEach(cleanup);

describe("useVoiceEnroll", () => {
  it("only allows finishing from 15 s of audio", async () => {
    commands.enrollVoiceLevel.mockResolvedValue(level(MIN_SECONDS - 1));
    const { result } = renderHook(() => useVoiceEnroll());
    await act(() => result.current.start());
    await waitFor(() => expect(result.current.state.seconds).toBe(MIN_SECONDS - 1));
    expect(result.current.canFinish).toBe(false);
    commands.enrollVoiceLevel.mockResolvedValue(level(MIN_SECONDS));
    await waitFor(() => expect(result.current.canFinish).toBe(true));
  });

  it("finishes by itself when the core says the buffer is full", async () => {
    const saved = vi.fn();
    commands.enrollVoiceLevel.mockResolvedValue(level(25, true));
    const { result } = renderHook(() => useVoiceEnroll(saved));
    await act(() => result.current.start());
    await waitFor(() => expect(result.current.state.phase).toBe("done"));
    expect(commands.voiceEnrollStop).toHaveBeenCalledTimes(1);
    expect(saved).toHaveBeenCalled();
  });

  it("keeps the core's code when saving fails, and the mic stays closed", async () => {
    commands.enrollVoiceLevel.mockResolvedValue(level(16));
    commands.voiceEnrollStop.mockResolvedValue({ status: "error", error: "tooShort" });
    const { result, unmount } = renderHook(() => useVoiceEnroll());
    await act(() => result.current.start());
    await waitFor(() => expect(result.current.canFinish).toBe(true));
    await act(() => result.current.finish());
    expect(result.current.state).toMatchObject({ phase: "idle", error: "tooShort" });
    unmount();
    expect(commands.voiceEnrollCancel).not.toHaveBeenCalled();
  });

  it("cancels an open enrollment when the screen goes", async () => {
    commands.enrollVoiceLevel.mockResolvedValue(level(3));
    const { result, unmount } = renderHook(() => useVoiceEnroll());
    await act(() => result.current.start());
    unmount();
    expect(commands.voiceEnrollCancel).toHaveBeenCalledTimes(1);
  });

  it("reports a failed start", async () => {
    commands.voiceEnrollStart.mockResolvedValue({ status: "error", error: "micPermission" });
    const { result } = renderHook(() => useVoiceEnroll());
    await act(() => result.current.start());
    expect(result.current.state).toMatchObject({ phase: "idle", error: "micPermission" });
  });
});
