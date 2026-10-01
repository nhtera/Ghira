// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Health } from "./health";
import { LiveBanners } from "./banners";
import { renderLive, setLive } from "./test-utils";
import { initialLive, useLive } from "../../state/live";
import type { CoreEvent } from "../../bindings";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("Health", () => {
  it("is quiet when all is well", () => {
    setLive({ state: "recording" });
    render(<Health />);
    expect(screen.getByRole("button", { name: /All good/ })).toBeTruthy();
  });

  it("suggests Fast mode past 3 s of lag", async () => {
    setLive({ state: "recording", asrLagS: 4.2 });
    render(<Health />);
    const toggle = screen.getByRole("button", { name: /4\.2 s behind/ });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    await userEvent.click(toggle);
    expect(screen.getByText("Switch to Fast mode")).toBeTruthy();
    expect(screen.getByRole("list").querySelector('[data-row="asr"]')?.getAttribute("data-warn")).toBe("true");
  });

  it("does not warn at 3 s", async () => {
    setLive({ state: "recording", asrLagS: 3 });
    render(<Health />);
    await userEvent.click(screen.getByRole("button"));
    expect(screen.queryByText("Switch to Fast mode")).toBeNull();
  });

  it("reports the audio route and a low disk", async () => {
    setLive({ state: "recording", aec: false, capture: { ...initialLive.capture, bluetoothHfp: true, diskLowBytes: 480_000_000 } });
    render(<Health />);
    await userEvent.click(screen.getByRole("button"));
    expect(screen.getByText(/Bluetooth headset/)).toBeTruthy();
    expect(screen.getByText(/Only 480 MB left/)).toBeTruthy();
  });
});

describe("LiveBanners", () => {
  const cap = (over: Partial<ReturnType<typeof useLive.getState>["capture"]>) => ({ ...initialLive.capture, ...over });

  it("shows the record-only and paused states", () => {
    setLive({ state: "paused", recordOnly: true });
    renderLive(<LiveBanners />);
    expect(screen.getByText(/Live transcript starts when speech models/)).toBeTruthy();
    expect(screen.getByText("Paused. Nothing is being recorded.")).toBeTruthy();
  });

  it("shows asleep, silent system, lost tracks, disk full", () => {
    setLive({ state: "recording", capture: cap({ asleep: true, systemSilent: true, lostTracks: [0, 1], diskFull: true }) });
    const { container } = renderLive(<LiveBanners />);
    for (const id of ["asleep", "system-silent", "mic-lost", "system-lost", "disk-full"]) expect(container.querySelector(`[data-banner="${id}"]`), id).not.toBeNull();
    expect(container.querySelector('[data-banner="disk-low"]')).toBeNull();
  });

  it("shows a low disk with the time left", () => {
    setLive({ state: "recording", capture: cap({ diskLowBytes: 240_000 * 45 }) });
    renderLive(<LiveBanners />);
    expect(screen.getByText(/about 45 minutes/)).toBeTruthy();
  });

  it("waits 10 s without levels, then clears when audio flows", () => {
    vi.useFakeTimers();
    setLive({ state: "recording" });
    const { container } = renderLive(<LiveBanners />);
    act(() => void vi.advanceTimersByTime(9_000));
    expect(container.querySelector('[data-banner="no-audio"]')).toBeNull();
    act(() => void vi.advanceTimersByTime(2_000));
    expect(container.querySelector('[data-banner="no-audio"]')).not.toBeNull();
    const level: CoreEvent = { seq: null, atMs: 0, event: { type: "levelMeter", meeting: "m", micDbfs: -20, systemDbfs: null } };
    act(() => useLive.getState().apply(level));
    expect(container.querySelector('[data-banner="no-audio"]')).toBeNull();
  });

  it("does not wait for audio while paused", () => {
    vi.useFakeTimers();
    setLive({ state: "paused" });
    const { container } = renderLive(<LiveBanners />);
    act(() => void vi.advanceTimersByTime(30_000));
    expect(container.querySelector('[data-banner="no-audio"]')).toBeNull();
  });
});
