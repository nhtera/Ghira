// SPDX-License-Identifier: Apache-2.0
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ipc } from "../ipc";
import { nextSpeech, usePlayer } from "./player";

describe("player store", () => {
  beforeEach(() => usePlayer.getState().unload());

  it("loads a play token for the meeting once", async () => {
    await usePlayer.getState().load("sample-0");
    const s = usePlayer.getState();
    expect(s.meeting).toBe("sample-0");
    expect(s.src).toBeTruthy();
    expect(s.durationMs).toBeGreaterThan(0);
    const src = s.src;
    await usePlayer.getState().load("sample-0");
    expect(usePlayer.getState().src).toBe(src);
  });

  it("reports a meeting without audio", async () => {
    await usePlayer.getState().load("no-such-meeting");
    expect(usePlayer.getState().error).toMatch(/not found/);
    expect(usePlayer.getState().src).toBeNull();
  });

  it("plays a span and asks to stop at its end", async () => {
    await usePlayer.getState().load("sample-0");
    usePlayer.getState().playSpan(10_000, 14_000);
    let s = usePlayer.getState();
    expect(s.seekRequest).toMatchObject({ ms: 10_000, play: true });
    expect(s.stopAtMs).toBe(14_000);
    const n = s.seekRequest!.n;
    usePlayer.getState().reportTime(12_000);
    expect(usePlayer.getState().seekRequest!.n).toBe(n);
    usePlayer.getState().reportTime(14_050);
    s = usePlayer.getState();
    expect(s.seekRequest).toMatchObject({ play: false });
    expect(s.stopAtMs).toBeNull();
  });

  it("gets a new token when the old one is refused, and resumes where it was", async () => {
    await usePlayer.getState().load("sample-0");
    usePlayer.setState({ currentMs: 7000, playing: true, error: "x" });
    const old = usePlayer.getState().src;
    vi.spyOn(ipc.commands, "issueAudioPlay").mockResolvedValueOnce({ status: "ok", data: { token: "fresh", durationMs: 99_000 } });
    vi.spyOn(ipc, "audioUrl").mockImplementation((t) => `url:${t}`);
    await usePlayer.getState().reissue();
    const s = usePlayer.getState();
    expect(s.src).toBe("url:fresh");
    expect(s.src).not.toBe(old);
    expect(s.error).toBeNull();
    expect(s.seekRequest).toMatchObject({ ms: 7000, play: true });
    vi.restoreAllMocks();
  });

  it("ignores a token answer that a newer request outran", async () => {
    let first!: (v: unknown) => void;
    const spy = vi.spyOn(ipc.commands, "issueAudioPlay");
    spy.mockReturnValueOnce(new Promise((r) => (first = r)) as never);
    spy.mockResolvedValueOnce({ status: "ok", data: { token: "second", durationMs: 1000 } });
    vi.spyOn(ipc, "audioUrl").mockImplementation((t) => `url:${t}`);
    const a = usePlayer.getState().load("sample-0");
    usePlayer.getState().unload(); // StrictMode: mount, unmount, mount
    await usePlayer.getState().load("sample-0");
    first({ status: "ok", data: { token: "first-revoked", durationMs: 1000 } });
    await a;
    expect(usePlayer.getState().src).toBe("url:second");
    vi.restoreAllMocks();
  });

  it("clamps seeks to the audio", async () => {
    await usePlayer.getState().load("sample-0");
    usePlayer.getState().seek(-5);
    expect(usePlayer.getState().currentMs).toBe(0);
    usePlayer.getState().seek(1e12);
    expect(usePlayer.getState().currentMs).toBe(usePlayer.getState().durationMs);
  });
});

describe("nextSpeech", () => {
  const spans = [
    { t0Ms: 0, t1Ms: 1000 },
    { t0Ms: 1500, t1Ms: 3000 },
    { t0Ms: 9000, t1Ms: 10_000 },
  ];
  it("skips only long gaps", () => {
    expect(nextSpeech(spans, 500)).toBeNull();
    expect(nextSpeech(spans, 1200)).toBeNull(); // 300 ms gap
    expect(nextSpeech(spans, 3100)).toBe(9000);
    expect(nextSpeech(spans, 10_500)).toBeNull();
  });
});
