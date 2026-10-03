// SPDX-License-Identifier: Apache-2.0
// The M2 screen's state as a reducer: the recording snapshot, the iOS shell
// events (phase, backlog, pocket, interruption, thermal) and the shared core
// events (lines, speakers, marks, level). `phase` is authoritative for what the
// screen shows; core `stateChanged` is never used to infer one.
import type {
  CoreEvent,
  LineInfo,
  MobileEvent,
  RecordOnlyReason,
  RecordPhase,
  RecordState,
  ResumePrompt,
  SpeakerInfo,
} from "../../bindings";

export type Line = {
  /** Store gid, or a local key for a line not persisted yet. */
  key: string;
  speaker: number | null;
  t0Ms: number;
  text: string;
  overlap: boolean;
  marked: boolean;
};

export type RecordModel = {
  phase: RecordPhase;
  meeting: string | null;
  elapsedS: number;
  /** Capture is running (the core's flag: true through loading and locked, false when paused or stopped). */
  recording: boolean;
  marks: number;
  /** When the marks were set (ms), to tell a repeated event from a new mark. */
  markTimes: number[];
  backlogS: number;
  /** The largest backlog since it last was empty (Catching up % counts down from it). */
  backlogPeakS: number;
  pocket: boolean;
  live: boolean;
  recordOnlyReason: RecordOnlyReason | null;
  thermal: number;
  /** The audio session was interrupted and nothing resumed it yet. */
  interruption: { call: boolean | null; recordedS: number | null } | null;
  /** A live line not final yet. */
  partial: string;
  lines: Line[];
  speakers: Record<number, SpeakerInfo>;
  /** Highest core event seq applied; older ones repeat what the snapshot has. */
  seq: number;
  /** Set when a new speaker turn starts; the screen reads it out once. */
  announce: { name: string; n: number } | null;
  /** The recording just ended and was saved. */
  saved: boolean;
};

export const initialModel: RecordModel = {
  phase: "idle",
  meeting: null,
  elapsedS: 0,
  recording: false,
  marks: 0,
  markTimes: [],
  backlogS: 0,
  backlogPeakS: 0,
  pocket: false,
  live: false,
  recordOnlyReason: null,
  thermal: 0,
  interruption: null,
  partial: "",
  lines: [],
  speakers: {},
  seq: 0,
  announce: null,
  saved: false,
};

export type Action =
  | { type: "snapshot"; state: RecordState }
  | { type: "mobile"; event: MobileEvent }
  | { type: "core"; env: CoreEvent }
  | { type: "prompt"; prompt: ResumePrompt }
  | { type: "reason"; reason: RecordOnlyReason | null }
  | { type: "tick" }
  | { type: "dismissSaved" };

/** Phases in which audio is being recorded and the clock runs. */
export const RECORDING: readonly RecordPhase[] = [
  "live",
  "locked",
  "catchingUp",
  "hot",
  "recordOnly",
];
/** A session exists (the transcript and controls are on screen). */
export const ACTIVE: readonly RecordPhase[] = [
  ...RECORDING,
  "paused",
  "interrupted",
  "finishing",
];

export const isRecording = (phase: RecordPhase) => RECORDING.includes(phase);
export const isActive = (phase: RecordPhase) => ACTIVE.includes(phase);

/**
 * Audio is being recorded now. `loading` with a session is recording too: the
 * models are still getting ready but the microphone is already on, so the clock
 * runs and Mark and Stop work.
 */
export const isCapturing = (m: Pick<RecordModel, "phase" | "meeting"> & { recording?: boolean }) => Boolean(m.recording) || isRecording(m.phase) || (m.phase === "loading" && m.meeting !== null);
/** A session is on screen (transcript, controls). */
export const hasSession = (m: Pick<RecordModel, "phase" | "meeting">) => isActive(m.phase) || (m.phase === "loading" && m.meeting !== null);

/** The line a mark at `tMs` belongs to: the last one that started by then. */
export function markLine(lines: Line[], tMs: number | null): Line[] {
  if (tMs === null) return lines;
  let at = -1;
  for (let i = lines.length - 1; i >= 0; i -= 1) {
    if (lines[i].t0Ms <= tMs) {
      at = i;
      break;
    }
  }
  if (at < 0 || lines[at].marked) return lines;
  return lines.map((l, i) => (i === at ? { ...l, marked: true } : l));
}

/** The shared levels window for the waveform. */
export const LEVELS = 40;
export const pushLevel = (levels: number[], db: number): number[] => [...levels, db].slice(-LEVELS);

/** 0..100: how much of the backlog the transcript has caught up with. */
export function catchUpPercent(
  m: Pick<RecordModel, "backlogS" | "backlogPeakS">,
): number {
  if (m.backlogPeakS <= 0) return 0;
  return Math.min(
    100,
    Math.max(0, Math.round((1 - m.backlogS / m.backlogPeakS) * 100)),
  );
}

const toLine = (l: LineInfo, marked = false): Line => ({
  key: l.gid || `t${l.t0Ms ?? 0}-${l.speaker ?? 0}`,
  speaker: l.speaker,
  t0Ms: l.t0Ms ?? 0,
  text: l.text,
  overlap: l.overlap,
  marked,
});

/** Avatar text of a speaker: the number of "Speaker 2", else the first letter. */
export function speakerInitial(label: string): string {
  return (
    label.match(/\d+\s*$/)?.[0].trim() ??
    Array.from(label.trim())[0]?.toUpperCase() ??
    "?"
  );
}

function appendLine(m: RecordModel, info: LineInfo): RecordModel {
  const line = toLine(info);
  if (m.lines.some((l) => l.key === line.key && info.gid)) return m;
  const prev = m.lines[m.lines.length - 1];
  const who = line.speaker === null ? undefined : m.speakers[line.speaker];
  // A provisional label ("Identifying…") is not read out.
  const name = who && !who.provisional ? who.label : null;
  const turn = name !== null && prev?.speaker !== line.speaker;
  return {
    ...m,
    lines: [...m.lines, line],
    partial: "",
    announce: turn ? { name, n: (m.announce?.n ?? 0) + 1 } : m.announce,
  };
}

function core(m: RecordModel, env: CoreEvent): RecordModel {
  if (env.seq !== null && env.seq <= m.seq) return m;
  const base = env.seq === null ? m : { ...m, seq: env.seq };
  const e = env.event;
  switch (e.type) {
    case "sessionStarted":
      return base.meeting === e.meeting ? base : { ...initialModel, phase: base.phase, meeting: e.meeting, seq: base.seq };
    case "speakerArrived":
    case "speakerConfirmed":
    case "speakerRenamed":
      return { ...base, speakers: { ...base.speakers, [e.speaker.id]: e.speaker } };
    case "transcriptPartial":
      return e.track === 0 ? { ...base, partial: e.text } : base;
    case "transcriptFinal":
      return appendLine(base, e.line);
    case "markAdded": {
      // The snapshot may already hold this mark.
      if (e.tMs !== null && base.markTimes.includes(e.tMs)) return base;
      return {
        ...base,
        marks: base.marks + 1,
        markTimes: e.tMs === null ? base.markTimes : [...base.markTimes, e.tMs],
        lines: markLine(base.lines, e.tMs),
      };
    }
    default:
      return base;
  }
}

function mobile(m: RecordModel, e: MobileEvent): RecordModel {
  switch (e.type) {
    case "phase": {
      const phase = e.phase;
      // Late events of a session that already ended: only a new start (loading) or the release (idle) count.
      if (m.phase === "done" && phase !== "idle" && phase !== "loading") return m;
      if (phase === "done") return { ...initialModel, phase, seq: m.seq, saved: true };
      if (phase === "idle") return { ...initialModel, seq: m.seq, saved: m.saved };
      return {
        ...m,
        phase,
        saved: false,
        recordOnlyReason: phase === "recordOnly" ? m.recordOnlyReason : null,
        recording: isRecording(phase) || (phase === "loading" && (m.recording || m.meeting !== null)),
        // Only an interruption keeps the question; any other phase answers it.
        interruption: phase === "interrupted" ? (m.interruption ?? { call: null, recordedS: m.elapsedS }) : null,
        backlogPeakS: phase === "catchingUp" ? m.backlogPeakS : 0,
      };
    }
    case "backlog": {
      const backlogS = e.backlogS ?? 0;
      return { ...m, backlogS, backlogPeakS: backlogS <= 0 ? 0 : Math.max(m.backlogPeakS, backlogS) };
    }
    case "pocket":
      return { ...m, pocket: e.muffled };
    case "thermal":
      return { ...m, thermal: e.level };
    case "interruption":
      // Ending does not resume: the screen keeps asking until the user answers.
      return e.began ? { ...m, interruption: { call: e.kind === "call", recordedS: m.elapsedS } } : m;
    default:
      return m;
  }
}

export function reducer(m: RecordModel, a: Action): RecordModel {
  switch (a.type) {
    case "snapshot": {
      const s = a.state;
      const session = s.session;
      const speakers: Record<number, SpeakerInfo> = {};
      for (const sp of session?.speakers ?? []) speakers[sp.id] = sp;
      const markTimes = (session?.marks ?? []).filter((t): t is number => t !== null);
      let lines = (session?.lines ?? []).map((l) => toLine(l));
      for (const t of markTimes) lines = markLine(lines, t);
      return {
        ...initialModel,
        phase: s.phase,
        meeting: session?.meeting ?? null,
        elapsedS: s.elapsedS ?? 0,
        recording: s.recording,
        marks: Math.max(s.marks, markTimes.length),
        markTimes,
        backlogS: s.backlogS ?? 0,
        backlogPeakS: s.phase === "catchingUp" ? (s.backlogS ?? 0) : 0,
        pocket: s.pocket,
        live: s.live,
        recordOnlyReason: s.recordOnlyReason,
        thermal: m.thermal,
        // Whether it was a call comes with the resume prompt.
        interruption: s.phase === "interrupted" ? { call: null, recordedS: s.elapsedS } : null,
        lines,
        speakers,
        seq: session?.seq ?? m.seq,
        saved: m.saved && s.phase === "idle",
      };
    }
    case "mobile":
      return mobile(m, a.event);
    case "core":
      return core(m, a.env);
    case "prompt":
      return a.prompt.pending ? { ...m, interruption: { call: a.prompt.call, recordedS: a.prompt.recordedS } } : m;
    case "reason":
      return { ...m, recordOnlyReason: a.reason };
    case "tick":
      return isCapturing(m) ? { ...m, elapsedS: m.elapsedS + 1 } : m;
    case "dismissSaved":
      return { ...m, saved: false };
  }
}
