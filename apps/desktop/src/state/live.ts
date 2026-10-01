// SPDX-License-Identifier: Apache-2.0
// The recording in progress, built from core events (live events go to this
// store, not to the query cache). `reduce` is pure, so a recorded event
// stream replays to the same state in tests.
import { create } from "zustand";
import type { CoreEvent, ErrorKind, LineInfo, SessionState, SpeakerInfo } from "../bindings";

export type LiveError = { kind: ErrorKind; message: string; atMs: number | null };

export type LiveState = {
  meeting: string | null;
  state: SessionState;
  /** Final lines in order. */
  lines: LineInfo[];
  /** The words in progress per track (0 mic, 1 system). */
  partial: Record<number, string>;
  speakers: Record<number, SpeakerInfo>;
  /** Marked moments (meeting ms). */
  marks: number[];
  asrLagS: number;
  aec: boolean;
  /** Mic and system levels (dBFS), when known. */
  levels: { mic: number | null; system: number | null };
  /** Recording without a live transcript (models missing). */
  recordOnly: boolean;
  errors: LiveError[];
  /** Last applied `seq`: a gap means events were missed (re-read state). */
  seq: number | null;
  /** Wall clock (unix ms) of the start, the current pause, and paused time so far. */
  startedAtMs: number | null;
  pausedAtMs: number | null;
  pausedTotalMs: number;
};

export const initialLive: LiveState = {
  meeting: null,
  state: "idle",
  lines: [],
  partial: {},
  speakers: {},
  marks: [],
  asrLagS: 0,
  aec: false,
  levels: { mic: null, system: null },
  recordOnly: false,
  errors: [],
  seq: null,
  startedAtMs: null,
  pausedAtMs: null,
  pausedTotalMs: 0,
};

/** Time recorded so far (excludes pauses), for the record control's clock. */
export function elapsedMs(s: Pick<LiveState, "startedAtMs" | "pausedAtMs" | "pausedTotalMs">, now: number): number {
  if (s.startedAtMs == null) return 0;
  return Math.max(0, (s.pausedAtMs ?? now) - s.startedAtMs - s.pausedTotalMs);
}

const ACTIVE: SessionState[] = ["starting", "recording", "paused", "stopping"];
export const isActive = (s: SessionState) => ACTIVE.includes(s);

export function reduce(s: LiveState, env: CoreEvent): LiveState {
  const e = env.event;
  const seq = env.seq ?? s.seq;
  // Events of another meeting (a job finishing an older one) don't touch the live view.
  if ("meeting" in e && e.meeting && s.meeting && e.meeting !== s.meeting && e.type !== "stateChanged") {
    return { ...s, seq };
  }
  switch (e.type) {
    case "stateChanged": {
      const at = env.atMs ?? Date.now();
      if (e.state === "starting") return { ...initialLive, meeting: e.meeting, state: e.state, seq };
      if (s.meeting && e.meeting !== s.meeting) return { ...s, seq };
      // The clock starts when audio does (not during the permission check).
      const timing =
        e.state === "recording" && s.startedAtMs == null
          ? { startedAtMs: at }
          : e.state === "paused"
          ? { pausedAtMs: s.pausedAtMs ?? at }
          : s.pausedAtMs != null
            ? { pausedAtMs: null, pausedTotalMs: s.pausedTotalMs + (at - s.pausedAtMs) }
            : {};
      return { ...s, ...timing, meeting: e.meeting, state: e.state, seq };
    }
    case "transcriptPartial":
      return { ...s, partial: { ...s.partial, [e.track]: e.text }, seq };
    case "transcriptFinal":
      // A final ends the words in progress (lines carry no track; the next
      // partial comes within a chunk).
      return { ...s, lines: [...s.lines, e.line], partial: {}, seq };
    case "speakerArrived":
    case "speakerConfirmed":
    case "speakerRenamed":
    case "speakerSplit":
      return { ...s, speakers: { ...s.speakers, [e.speaker.id]: e.speaker }, seq };
    case "speakersMerged": {
      const speakers = { ...s.speakers };
      delete speakers[e.from];
      const lines = s.lines.map((l) => (l.speaker === e.from ? { ...l, speaker: e.into } : l));
      return { ...s, speakers, lines, seq };
    }
    case "speakerNotAPerson": {
      const sp = s.speakers[e.id];
      return sp ? { ...s, speakers: { ...s.speakers, [e.id]: { ...sp, notPerson: true } }, seq } : { ...s, seq };
    }
    case "markAdded":
      return { ...s, marks: [...s.marks, e.tMs ?? 0], seq };
    case "discardApplied": {
      const from = e.fromMs ?? 0;
      return {
        ...s,
        lines: s.lines.filter((l) => (l.t1Ms ?? 0) <= from),
        marks: s.marks.filter((m) => m < from),
        partial: {},
        seq,
      };
    }
    case "levelMeter":
      return { ...s, levels: { mic: e.micDbfs, system: e.systemDbfs }, seq };
    case "health":
      return { ...s, asrLagS: e.asrLagS ?? 0, aec: e.aec, seq };
    case "error":
      return {
        ...s,
        recordOnly: s.recordOnly || e.kind === "modelsMissing",
        errors: [...s.errors, { kind: e.kind, message: e.message, atMs: env.atMs }],
        seq,
      };
    case "jobProgress":
    case "notesReady":
      return { ...s, seq };
  }
}

type LiveStore = LiveState & { apply: (e: CoreEvent) => void; reset: () => void };

export const useLive = create<LiveStore>((set) => ({
  ...initialLive,
  apply: (e) => set((s) => reduce(s, e)),
  reset: () => set(initialLive),
}));
