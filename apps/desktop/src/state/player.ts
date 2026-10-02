// SPDX-License-Identifier: Apache-2.0
// The meeting detail's audio player (D6): one `<audio>` element, owned by the
// audio bar, playing the open meeting through a ghi-audio play token. Anything
// else (citation chips, transcript lines, the topic rail) asks this store to
// seek or play; the audio bar carries the request out and reports the time
// back. Times are meeting ms.
import { create } from "zustand";
import { ipc } from "../ipc";

export const RATES = [0.75, 1, 1.25, 1.5, 2] as const;
export type Rate = (typeof RATES)[number];

/** A request for the `<audio>` owner; `n` makes repeats distinct. */
export type SeekRequest = { ms: number; play: boolean; n: number };

export interface PlayerState {
  meeting: string | null;
  /** The `<audio>` source (null until loaded, or when there is no audio). */
  src: string | null;
  durationMs: number;
  currentMs: number;
  playing: boolean;
  rate: Rate;
  /** Skip the gaps between speech (> 1.5 s). */
  skipSilence: boolean;
  loading: boolean;
  error: string | null;
  /** Stop when playback reaches this time (a citation's end), then clear. */
  stopAtMs: number | null;
  seekRequest: SeekRequest | null;
  /** Gets a play token for `meeting` (no-op if it's already loaded). */
  load(meeting: string): Promise<void>;
  unload(): void;
  /** Gets a fresh play token (the old one expired or was revoked) and resumes where it was. */
  reissue(): Promise<void>;
  /** Moves to `ms`; `play` also starts playing. */
  seek(ms: number, play?: boolean): void;
  /** Plays `t0..t1` (a citation, a transcript line) and stops at its end. */
  playSpan(t0Ms: number, t1Ms: number): void;
  toggle(): void;
  setRate(rate: Rate): void;
  setSkipSilence(on: boolean): void;
  /** From the audio bar: the element's time / state. */
  reportTime(ms: number): void;
  reportPlaying(playing: boolean): void;
  reportError(message: string): void;
}

const IDLE = {
  meeting: null,
  src: null,
  durationMs: 0,
  currentMs: 0,
  playing: false,
  loading: false,
  error: null,
  stopAtMs: null,
  seekRequest: null,
} as const;

let requests = 0;
/** Counts token requests: only the latest one may apply its answer (StrictMode, A → B → A). */
let issues = 0;

export const usePlayer = create<PlayerState>((set, get) => ({
  ...IDLE,
  rate: 1,
  skipSilence: false,
  load: async (meeting) => {
    if (get().meeting === meeting && (get().src || get().loading)) return;
    const issue = ++issues;
    set({ ...IDLE, meeting, loading: true });
    const r = await ipc.commands.issueAudioPlay(meeting);
    if (issue !== issues) return; // a newer load, a reissue or an unload came meanwhile (its token revoked this one)
    if (r.status === "error") set({ loading: false, error: r.error });
    else set({ loading: false, error: null, src: ipc.audioUrl(r.data.token), durationMs: r.data.durationMs ?? 0 });
  },
  unload: () => {
    issues++;
    set({ ...IDLE });
  },
  reissue: async () => {
    const { meeting, currentMs, playing } = get();
    if (!meeting) return;
    const issue = ++issues;
    const r = await ipc.commands.issueAudioPlay(meeting);
    if (issue !== issues) return;
    if (r.status === "error") return set({ error: r.error, playing: false });
    set({ src: ipc.audioUrl(r.data.token), error: null, durationMs: r.data.durationMs ?? get().durationMs, seekRequest: { ms: currentMs, play: playing, n: ++requests } });
  },
  seek: (ms, play = false) => {
    const clamped = Math.max(0, get().durationMs ? Math.min(ms, get().durationMs) : ms);
    set({ currentMs: clamped, stopAtMs: null, seekRequest: { ms: clamped, play, n: ++requests } });
  },
  playSpan: (t0Ms, t1Ms) => {
    get().seek(t0Ms, true);
    set({ stopAtMs: t1Ms > t0Ms ? t1Ms : null });
  },
  toggle: () => {
    const { playing, currentMs } = get();
    set({ seekRequest: { ms: currentMs, play: !playing, n: ++requests }, stopAtMs: null });
  },
  setRate: (rate) => set({ rate }),
  setSkipSilence: (skipSilence) => set({ skipSilence }),
  reportTime: (ms) => {
    const stop = get().stopAtMs;
    if (stop != null && ms >= stop) {
      set({ currentMs: ms, stopAtMs: null, seekRequest: { ms, play: false, n: ++requests } });
    } else set({ currentMs: ms });
  },
  reportPlaying: (playing) => set({ playing }),
  reportError: (error) => set({ error, playing: false }),
}));

/**
 * Where to jump when skipping silence: from `ms`, the start of the next
 * speech if the gap to it is longer than `minGapMs`; else null (keep going).
 * `spans` are speech spans in time order.
 */
export function nextSpeech(spans: readonly { t0Ms: number; t1Ms: number }[], ms: number, minGapMs = 1500): number | null {
  if (spans.some((s) => s.t0Ms <= ms && ms < s.t1Ms)) return null;
  const next = spans.find((s) => s.t0Ms > ms);
  if (!next) return null;
  return next.t0Ms - ms > minGapMs ? next.t0Ms : null;
}
