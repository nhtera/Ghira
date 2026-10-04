// SPDX-License-Identifier: Apache-2.0
// The recording in progress, built from core events (live events go to this
// store, not to the query cache). `reduce` is pure, so a recorded event
// stream replays to the same state in tests.
import { create } from "zustand";
import type { CoreEvent, ErrorKind, LineInfo, SessionSnapshot, SessionState, SpeakerInfo } from "../bindings";

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
  /** What was started: mode, language hint (null = both), title. */
  session: { mode: string; language: string | null; title: string; consentConfirmed: boolean; sensitive: boolean } | null;
  /** Capture conditions shown as health notices (D4, D12). */
  capture: {
    /** The Mac slept mid-recording (the gap is marked; recording resumes on wake). */
    asleep: boolean;
    /** The system track is digital silence: system-audio access denied or broken. */
    systemSilent: boolean;
    /** Free disk bytes when low, `null` when fine. */
    diskLowBytes: number | null;
    diskFull: boolean;
    /** Tracks whose device went away (0 mic, 1 system). */
    lostTracks: number[];
    /** The input is a Bluetooth headset in call mode (16 kHz quality). */
    bluetoothHfp: boolean;
  };
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
  session: null,
  capture: { asleep: false, systemSilent: false, diskLowBytes: null, diskFull: false, lostTracks: [], bluetoothHfp: false },
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

/**
 * The live state from a snapshot (a reloaded webview, a second window). The
 * clock restarts from the snapshot's meeting time.
 */
export function fromSnapshot(snap: SessionSnapshot, nowWall: number): LiveState {
  return {
    ...initialLive,
    meeting: snap.meeting,
    state: snap.state,
    lines: snap.lines,
    speakers: Object.fromEntries(snap.speakers.map((sp) => [sp.id, sp])),
    marks: snap.marks.filter((m): m is number => m != null),
    recordOnly: !snap.transcribing,
    session: { mode: snap.mode, language: snap.language, title: snap.title, consentConfirmed: snap.consentConfirmed, sensitive: snap.sensitive },
    seq: snap.seq,
    startedAtMs: nowWall - (snap.nowMs ?? 0),
    pausedAtMs: snap.state === "paused" ? nowWall : null,
  };
}

export function reduce(s: LiveState, env: CoreEvent): LiveState {
  const e = env.event;
  // Already in the state (a snapshot raced with the event stream).
  if (env.seq != null && s.seq != null && env.seq <= s.seq) return s;
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
    case "sessionStarted":
      return { ...s, session: { mode: e.mode, language: e.language, title: e.title, consentConfirmed: false, sensitive: false }, seq };
    case "transcriptPartial":
      return { ...s, partial: { ...s.partial, [e.track]: e.text }, seq };
    case "transcriptFinal":
      // A final ends the words in progress (lines carry no track; the next
      // partial comes within a chunk). A line already known (by gid) is skipped.
      if (e.line.gid && s.lines.some((l) => l.gid === e.line.gid)) return { ...s, seq };
      return { ...s, lines: [...s.lines, e.line], partial: {}, seq };
    case "speakerArrived":
    case "speakerConfirmed":
    case "speakerRenamed":
      return { ...s, speakers: { ...s.speakers, [e.speaker.id]: e.speaker }, seq };
    case "speakerSplit": {
      // The moved lines go to the new speaker (every window sees the same).
      const moved = new Set(e.lines);
      const lines = s.lines.map((l) => (moved.has(l.gid) ? { ...l, speaker: e.speaker.id } : l));
      return { ...s, speakers: { ...s.speakers, [e.speaker.id]: e.speaker }, lines, seq };
    }
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
    case "sensitiveChanged":
      return { ...s, session: s.session ? { ...s.session, sensitive: e.sensitive } : s.session, seq };
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
    case "slept":
    case "woke":
      return { ...s, capture: { ...s.capture, asleep: e.type === "slept" }, seq };
    case "silentSystemTrack":
    case "systemAudioRestarted":
      return { ...s, capture: { ...s.capture, systemSilent: e.type === "silentSystemTrack" }, seq };
    case "diskLow":
      return { ...s, capture: { ...s.capture, diskLowBytes: e.freeBytes }, seq };
    case "diskFull":
      return { ...s, capture: { ...s.capture, diskFull: true }, seq };
    case "trackLost":
      return { ...s, capture: { ...s.capture, lostTracks: [...new Set([...s.capture.lostTracks, e.track])] }, seq };
    case "routeChanged":
      return { ...s, capture: { ...s.capture, bluetoothHfp: e.bluetoothHfp }, seq };
    // appAudioFallback / capture retry events: shown by the system banners, not the live state.
    case "jobProgress":
    case "notesReady":
    case "appAudioFallback":
    case "captureRecovered":
    case "captureRetryFailed":
      return { ...s, seq };
  }
}

type LiveStore = LiveState & {
  apply: (e: CoreEvent) => void;
  restore: (snap: SessionSnapshot) => void;
  reset: () => void;
  /** After a successful setMeetingTitle / setConsentConfirmed. */
  setSessionTitle: (title: string) => void;
  setSessionConsent: (confirmed: boolean) => void;
};

export const useLive = create<LiveStore>((set) => ({
  ...initialLive,
  apply: (e) => set((s) => reduce(s, e)),
  restore: (snap) => set(fromSnapshot(snap, Date.now())),
  reset: () => set(initialLive),
  setSessionTitle: (title) => set((s) => (s.session ? { session: { ...s.session, title } } : {})),
  setSessionConsent: (consentConfirmed) => set((s) => (s.session ? { session: { ...s.session, consentConfirmed } } : {})),
}));
