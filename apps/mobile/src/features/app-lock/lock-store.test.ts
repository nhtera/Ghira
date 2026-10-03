// SPDX-License-Identifier: Apache-2.0
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LOCKED_EVENT, UNLOCKED_EVENT } from "./events";
import { getLockSnapshot, markUnlocked, pageVisibility, refreshLock, resetLockStore, setLockEnabled } from "./lock-store";

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
