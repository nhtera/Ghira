// SPDX-License-Identifier: Apache-2.0
// The screen's feed: a snapshot that fails, hangs or overlaps another must
// never leave the events held (the transcript and waveform would freeze while
// the clock goes on).
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CoreEvent, MobileEvent, RecordState } from "../../bindings";

const ok = <T,>(data: T) => ({ status: "ok" as const, data });
const hooks = vi.hoisted(() => ({
  core: null as null | ((e: CoreEvent) => void),
  mobile: null as null | ((e: MobileEvent) => void),
  recordSnapshot: vi.fn(),
  recordResumePrompt: vi.fn(),
}));
vi.mock("../../ipc", () => ({
  ipc: {
    commands: { recordSnapshot: hooks.recordSnapshot, recordResumePrompt: hooks.recordResumePrompt },
    onCoreEvent: async (cb: (e: CoreEvent) => void) => {
      hooks.core = cb;
      return () => {};
    },
    onMobileEvent: async (cb: (e: MobileEvent) => void) => {
      hooks.mobile = cb;
      return () => {};
    },
  },
}));

import { useRecord, type RecordSetup } from "./use-record";

const setup: RecordSetup = { callActive: false, mic: "granted", recordOnlyDevice: false, target: "phone", language: "auto" };
const snapshot = (seq: number, lines: string[] = []): RecordState => ({
  phase: "live",
  recording: true,
  elapsedS: 5,
  marks: 0,
  levelDb: -20,
  backlogS: null,
  catchUpX: null,
  pocket: false,
  live: true,
  recordOnlyReason: null,
  session: {
    meeting: "m-1",
    seq,
    sensitive: false,
    speakers: [],
    marks: [],
    lines: lines.map((text, i) => ({ gid: `g${i}`, speaker: null, t0Ms: i * 1000, t1Ms: i * 1000 + 500, text, overlap: false, words: [] })),
  } as unknown as RecordState["session"],
});
const partial = (seq: number, text: string): CoreEvent => ({ seq, atMs: null, event: { type: "transcriptPartial", meeting: "m-1", track: 0, text } });
const final = (seq: number, gid: string, text: string): CoreEvent => ({
  seq,
  atMs: null,
  event: { type: "transcriptFinal", track: 0, meeting: "m-1", line: { gid, speaker: null, t0Ms: seq * 100, t1Ms: seq * 100 + 50, text, overlap: false, words: [] } },
});
const wake = () => {
  Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
  document.dispatchEvent(new Event("visibilitychange"));
};

beforeEach(() => {
  hooks.core = null;
  hooks.mobile = null;
  hooks.recordSnapshot.mockReset();
  hooks.recordResumePrompt.mockReset();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("useRecord feed", () => {
  it("shows events after a snapshot that failed", async () => {
    hooks.recordSnapshot.mockRejectedValue(new TypeError("Load failed"));
    const { result } = renderHook(() => useRecord(setup));
    await waitFor(() => expect(hooks.core).not.toBeNull());
    await act(async () => {
      hooks.core!(partial(10, "chốt scope cho"));
    });
    await waitFor(() => expect(result.current.model.partial).toBe("chốt scope cho"));
  });

  it("asks for the snapshot again after a failed one", async () => {
    hooks.recordSnapshot.mockRejectedValueOnce(new TypeError("Load failed")).mockResolvedValue(ok(snapshot(7, ["một", "hai"])));
    const { result } = renderHook(() => useRecord(setup));
    await waitFor(() => expect(result.current.model.lines).toHaveLength(2), { timeout: 4000 });
  });

  it("does not hold events for ever when the snapshot never answers", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    hooks.recordSnapshot.mockReturnValue(new Promise(() => {}));
    const { result } = renderHook(() => useRecord(setup));
    await vi.waitFor(() => expect(hooks.core).not.toBeNull());
    await act(async () => {
      hooks.core!(partial(10, "vẫn nghe"));
      await vi.advanceTimersByTimeAsync(15_000);
    });
    expect(result.current.model.partial).toBe("vẫn nghe");
  });

  it("lets events through once two overlapping snapshots are done", async () => {
    const slow: ((v: unknown) => void)[] = [];
    hooks.recordSnapshot.mockImplementation(() => new Promise((r) => slow.push(r)));
    const { result } = renderHook(() => useRecord(setup));
    await waitFor(() => expect(slow).toHaveLength(1));
    // The webview wakes again while the first snapshot is still being read.
    act(() => wake());
    await act(async () => slow[0](ok(snapshot(10, ["cũ"]))));
    // The second read follows the first, not alongside it.
    await waitFor(() => expect(slow).toHaveLength(2));
    await act(async () => slow[1](ok(snapshot(19, ["cũ", "mới"]))));
    await waitFor(() => expect(result.current.model.lines.map((l) => l.text)).toEqual(["cũ", "mới"]));
    await act(async () => {
      hooks.core!(partial(20, "sau khi đọc xong"));
    });
    expect(result.current.model.partial).toBe("sau khi đọc xong");
  });

  it("asks for a failing snapshot at most four times in all", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    hooks.recordSnapshot.mockRejectedValue(new TypeError("Load failed"));
    renderHook(() => useRecord(setup));
    await vi.waitFor(() => expect(hooks.recordSnapshot).toHaveBeenCalled());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(hooks.recordSnapshot).toHaveBeenCalledTimes(4);
  });

  it("replays every event held during a failed read, in order", async () => {
    let fail: (e: unknown) => void = () => {};
    hooks.recordSnapshot.mockImplementationOnce(() => new Promise((_, rej) => (fail = rej))).mockRejectedValue(new TypeError("Load failed"));
    const { result } = renderHook(() => useRecord(setup));
    await waitFor(() => expect(hooks.core).not.toBeNull());
    await act(async () => {
      hooks.core!(final(5, "a", "một"));
      hooks.core!(final(30, "b", "hai"));
    });
    expect(result.current.model.lines).toHaveLength(0);
    await act(async () => fail(new TypeError("Load failed")));
    expect(result.current.model.lines.map((l) => l.text)).toEqual(["một", "hai"]);
  });

  it("replays only the events newer than the snapshot it did get", async () => {
    let answer: (v: unknown) => void = () => {};
    hooks.recordSnapshot.mockImplementation(() => new Promise((r) => (answer = r)));
    const { result } = renderHook(() => useRecord(setup));
    await waitFor(() => expect(hooks.core).not.toBeNull());
    await act(async () => {
      hooks.core!(final(5, "old", "đã có trong ảnh chụp"));
      hooks.core!(final(30, "new", "sau ảnh chụp"));
    });
    await act(async () => answer(ok(snapshot(10, ["đã có"]))));
    expect(result.current.model.lines.map((l) => l.text)).toEqual(["đã có", "sau ảnh chụp"]);
  });
});
