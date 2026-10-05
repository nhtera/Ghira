// SPDX-License-Identifier: Apache-2.0
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LOCKED_EVENT, UNLOCKED_EVENT } from "./events";
import { getLockSnapshot, markUnlocked, pageVisibility, refreshLock, resetLockStore, retryStartup, setLockEnabled } from "./lock-store";

const mock = () => window.__ghiSettingsMock!;

describe("lock store", () => {
  beforeEach(async () => {
    await import("../../ipc");
    vi.useFakeTimers();
    mock().reset();
    resetLockStore();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("retries with a growing pause while the app is starting", async () => {
    mock().starting = true;
    await refreshLock();
    expect(getLockSnapshot().phase).toBe("unknown");
    await vi.advanceTimersByTimeAsync(400);
    expect(getLockSnapshot().phase).toBe("unknown");
    mock().starting = false;
    await vi.advanceTimersByTimeAsync(800);
    expect(getLockSnapshot().phase).toBe("unlocked");
  });

  it("names the reason when the store can't be opened, and stops asking", async () => {
    mock().storeProblem = "keyMissing";
    await refreshLock();
    expect(getLockSnapshot()).toMatchObject({ phase: "unavailable", problem: "keyMissing" });
    const asked = mock().calls.lockState;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(mock().calls.lockState).toBe(asked);
  });

  it("does not open the store again on a focus; only Try again does", async () => {
    mock().storeProblem = "keyLocked";
    await refreshLock();
    const asked = mock().calls.lockState;
    await refreshLock();
    expect(mock().calls.lockState).toBe(asked);
    await retryStartup();
    expect(mock().calls.lockState).toBe(asked + 1);
  });

  it("a store that opened but failed to start is the generic failed state", async () => {
    mock().storeProblem = "startup";
    await refreshLock();
    expect(getLockSnapshot()).toMatchObject({ phase: "failed", problem: null });
  });

  it("carries on once \"Try again\" finds the store", async () => {
    mock().storeProblem = "damaged";
    await refreshLock();
    expect(getLockSnapshot().phase).toBe("unavailable");
    await retryStartup();
    expect(getLockSnapshot().phase).toBe("unavailable");
    mock().storeProblem = null;
    await retryStartup();
    expect(getLockSnapshot()).toMatchObject({ phase: "unlocked", problem: null });
  });

  it("shows \"failed\" instead of nothing when startup keeps failing", async () => {
    mock().startupFails = true;
    await refreshLock();
    for (const ms of [400, 800, 1600, 3200]) {
      expect(getLockSnapshot().phase).toBe("unknown");
      await vi.advanceTimersByTimeAsync(ms);
    }
    expect(getLockSnapshot()).toMatchObject({ phase: "failed", problem: null });
    // It keeps trying in the background and recovers by itself.
    mock().startupFails = false;
    await vi.advanceTimersByTimeAsync(4000);
    expect(getLockSnapshot().phase).toBe("unlocked");
  });

  it("announces a lock and an unlock on window", async () => {
    const seen: string[] = [];
    const log = (e: Event) => seen.push(e.type);
    window.addEventListener(LOCKED_EVENT, log);
    window.addEventListener(UNLOCKED_EVENT, log);
    mock().locked = true;
    await refreshLock();
    expect(getLockSnapshot().phase).toBe("locked");
    markUnlocked();
    expect(seen).toEqual([LOCKED_EVENT, UNLOCKED_EVENT]);
    window.removeEventListener(LOCKED_EVENT, log);
    window.removeEventListener(UNLOCKED_EVENT, log);
  });

  it("covers a hidden page only while the app lock is on", async () => {
    await refreshLock();
    pageVisibility(false);
    expect(getLockSnapshot().covered).toBe(false);
    setLockEnabled(true);
    pageVisibility(false);
    expect(getLockSnapshot().covered).toBe(true);
    pageVisibility(true);
    expect(getLockSnapshot().covered).toBe(false);
  });
});
