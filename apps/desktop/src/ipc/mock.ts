// SPDX-License-Identifier: Apache-2.0
// A scripted core for the browser: the same command and event types as the
// real one (bindings.ts), replaying the sample meeting (brief §10) as a live
// session. Times are compressed: one transcript line per `LINE_MS`.
import sample from "@ghi/ui/mocks/sample-meeting.json";
import type { AppVersion, CoreEvent, Event, MenuAction, RecordMode, SpeakerInfo, Stage } from "../bindings";
import type { Commands, Ipc } from "./ipc";

const LINE_MS = 1800;
// Palette slots in assignment order (s1, s2, s4, s8: tokens speakerOrder).
const SLOTS = [1, 2, 4, 8];

type Result<T> = { status: "ok"; data: T } | { status: "error"; error: string };
const ok = <T>(data: T): Promise<Result<T>> => Promise.resolve({ status: "ok", data });
const fail = <T>(error: string): Promise<Result<T>> => Promise.resolve({ status: "error", error });

const listeners = new Set<(e: CoreEvent) => void>();
const menuListeners = new Set<(a: MenuAction) => void>();
let seq = 0;
const emit = (event: Event) => {
  const env: CoreEvent = { seq: seq++, atMs: Date.now(), event };
  for (const l of listeners) l(env);
};

type Session = {
  id: string;
  mode: RecordMode;
  startedAt: number;
  pausedMs: number;
  pausedAt: number | null;
  timers: number[];
  /** Next transcript line to play. */
  next: number;
  arrived: Set<number>;
};
let session: Session | null = null;
let meetings = 0;

const nowMs = (s: Session) => (s.pausedAt ?? Date.now()) - s.startedAt - s.pausedMs;

function speaker(i: number, mode: RecordMode): SpeakerInfo {
  const me = i === 0 && mode === "call";
  return {
    id: i + 1,
    label: me ? "Me" : `Speaker ${i + 1}`,
    colorSlot: SLOTS[i % SLOTS.length],
    isMe: me,
    provisional: false,
    notPerson: false,
    others: false,
  };
}

function playLine(s: Session) {
  const line = sample.transcript[s.next];
  if (!line) return;
  const at = nowMs(s);
  if (!s.arrived.has(line.s)) {
    s.arrived.add(line.s);
    emit({ type: "speakerArrived", meeting: s.id, speaker: speaker(line.s, s.mode) });
  }
  const words = line.x.split(" ");
  emit({ type: "transcriptPartial", meeting: s.id, track: line.s === 0 ? 0 : 1, text: words.slice(0, 3).join(" ") });
  const step = (LINE_MS * 0.8) / words.length;
  emit({
    type: "transcriptFinal",
    meeting: s.id,
    line: {
      gid: `${s.id}-l${s.next}`,
      speaker: line.s + 1,
      t0Ms: Math.max(0, at - LINE_MS * 0.8),
      t1Ms: at,
      text: line.x,
      overlap: false,
      words: words.map((w, i) => ({
        text: w,
        t0Ms: at - LINE_MS * 0.8 + i * step,
        t1Ms: at - LINE_MS * 0.8 + (i + 1) * step,
        lowConfidence: "low" in line && line.low === w,
      })),
    },
  });
  emit({ type: "health", meeting: s.id, asrLagS: 0.4, asrSkippedS: 0, aec: s.mode === "call" });
  s.next += 1;
}

function tick(s: Session) {
  if (session !== s) return;
  if (s.pausedAt == null) {
    playLine(s);
    emit({ type: "levelMeter", meeting: s.id, micDbfs: -18 - Math.random() * 12, systemDbfs: s.mode === "call" ? -20 - Math.random() * 14 : null });
  }
  if (s.next < sample.transcript.length) s.timers.push(window.setTimeout(() => tick(s), LINE_MS));
}

const STAGES: Stage[] = ["decoding", "refiningSpeakers", "matchingVoices", "improvingTranscript", "writingNotes"];

function process(id: string) {
  STAGES.forEach((stage, i) => {
    window.setTimeout(() => emit({ type: "jobProgress", meeting: id, job: 1, kind: "final_pass", stage, progress: 1 }), 400 * (i + 1));
  });
  window.setTimeout(() => {
    emit({ type: "notesReady", meeting: id, version: 2 });
    emit({ type: "stateChanged", meeting: id, state: "ready" });
  }, 400 * (STAGES.length + 1));
}

const commands: Commands = {
  appVersion: () => Promise.resolve<AppVersion>({ app: "0.1.0", core: "0.1.0 (mock)" }),
  startRecording: (mode) => {
    if (session) return fail("a recording is already running");
    const id = `mock-${++meetings}`;
    session = { id, mode, startedAt: Date.now(), pausedMs: 0, pausedAt: null, timers: [], next: 0, arrived: new Set() };
    emit({ type: "stateChanged", meeting: id, state: "starting" });
    emit({ type: "stateChanged", meeting: id, state: "recording" });
    const s = session;
    s.timers.push(window.setTimeout(() => tick(s), 600));
    return ok(id);
  },
  stopRecording: () => {
    const s = session;
    if (!s) return fail("nothing is recording");
    s.timers.forEach(clearTimeout);
    const durationMs = nowMs(s);
    session = null;
    emit({ type: "stateChanged", meeting: s.id, state: "stopping" });
    emit({ type: "stateChanged", meeting: s.id, state: "processing" });
    process(s.id);
    return ok({ meeting: s.id, durationMs });
  },
  pauseRecording: () => {
    if (!session) return fail("nothing is recording");
    session.pausedAt ??= Date.now();
    emit({ type: "stateChanged", meeting: session.id, state: "paused" });
    return ok(null);
  },
  resumeRecording: () => {
    if (!session) return fail("nothing is recording");
    if (session.pausedAt != null) session.pausedMs += Date.now() - session.pausedAt;
    session.pausedAt = null;
    emit({ type: "stateChanged", meeting: session.id, state: "recording" });
    return ok(null);
  },
  markMoment: () => {
    if (!session) return fail("nothing is recording");
    const tMs = nowMs(session);
    emit({ type: "markAdded", meeting: session.id, tMs });
    return ok(tMs);
  },
  discardLast: (seconds) => {
    if (!session) return fail("nothing is recording");
    const fromMs = Math.max(0, nowMs(session) - (seconds ?? 0) * 1000);
    emit({ type: "discardApplied", meeting: session.id, fromMs });
    return ok(fromMs);
  },
  renameSpeaker: (id, name) => {
    if (!session) return fail("nothing is recording");
    emit({ type: "speakerRenamed", meeting: session.id, speaker: { ...speaker(id - 1, session.mode), label: name } });
    return ok(null);
  },
  mergeSpeakers: (from, into) => {
    if (!session) return fail("nothing is recording");
    emit({ type: "speakersMerged", meeting: session.id, from, into });
    return ok(null);
  },
  splitSpeaker: () => (session ? ok(null) : fail("nothing is recording")),
  speakerNotAPerson: (id) => {
    if (!session) return fail("nothing is recording");
    emit({ type: "speakerNotAPerson", meeting: session.id, id });
    return ok(null);
  },
  importRecording: () => ok({ meeting: `mock-${++meetings}`, duplicate: false, durationMs: 60_000 }),
};

export const mockIpc: Ipc = {
  kind: "mock",
  commands,
  onCoreEvent: (cb) => {
    listeners.add(cb);
    return Promise.resolve(() => void listeners.delete(cb));
  },
  onMenuAction: (cb) => {
    menuListeners.add(cb);
    return Promise.resolve(() => void menuListeners.delete(cb));
  },
};

