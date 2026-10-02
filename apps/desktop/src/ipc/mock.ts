// SPDX-License-Identifier: Apache-2.0
// A scripted core for the browser: the same command and event types as the
// real one (bindings.ts), replaying the sample meeting (brief §10) as a live
// session. Times are compressed: one transcript line per `LINE_MS`.
import library from "@ghi/ui/mocks/library.json";
import sample from "@ghi/ui/mocks/sample-meeting.json";
import type {
  AppSettings,
  AppVersion,
  CoreEvent,
  Event,
  MeetingDetected,
  MeetingRow,
  MenuAction,
  ModelDownload,
  Navigate,
  NoteLine,
  QuitRequested,
  RecordMode,
  SpeakerInfo,
  UpdateChanged,
  UpdateStatus,
  LockChanged,
  Stage,
} from "../bindings";
import type { Commands, Ipc } from "./ipc";
import { audioUrl, namesOf, onImportStaged, onImportUpdate, reviewCommands, simulateImportDrop, transcriptOf } from "./mock-review";
import { aiCommands } from "./mock-ai";
import { peopleCommands } from "./mock-people";

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

function setRow(id: string, patch: Partial<MeetingRow>) {
  const r = rows.find((x) => x.gid === id);
  if (r) Object.assign(r, patch);
}

function process(id: string) {
  setRow(id, { status: "processing", job: { kind: "final_pass", progress: 0, waitingForModels: false } });
  STAGES.forEach((stage, i) => {
    window.setTimeout(() => emit({ type: "jobProgress", meeting: id, job: 1, kind: "final_pass", stage, progress: 1 }), 400 * (i + 1));
  });
  window.setTimeout(() => {
    setRow(id, { status: "ready", job: null, transcriptVersion: 2 });
    emit({ type: "notesReady", meeting: id, version: 2 });
    emit({ type: "stateChanged", meeting: id, state: "ready" });
  }, 400 * (STAGES.length + 1));
}

// Library: the design's sample rows, plus meetings recorded in this session.
const STATUS: Record<string, string> = { ready: "ready", cloud: "ready", final: "processing", needs: "ready", failed: "failed" };
const rows: MeetingRow[] = library.rows.map((r, i) => ({
  gid: `sample-${i}`,
  title: r.ti,
  startedAt: Date.now() - (r.g + 1) * 86_400_000,
  durationMs: (parseInt(r.dur) || 30) * 60_000,
  source: r.src,
  mode: r.src === "room" ? "room" : "call",
  status: STATUS[r.st] ?? "ready",
  transcriptVersion: 2,
  cloudUsed: r.st === "cloud",
  consentConfirmed: false,
  template: null,
  people: r.ppl.filter((p) => p !== "Me").map((name, k) => ({ name, colorSlot: SLOTS[(k + 1) % SLOTS.length]! })),
  job: r.st === "final" ? { kind: "final_pass", progress: (("pct" in r ? r.pct : 0) ?? 0) / 100, waitingForModels: false } : null,
}));
const notes = new Map<string, NoteLine[]>();
// Onboarding is skipped on the mock so the shell opens straight away.
let settings: AppSettings = {
  onboardingDone: true,
  detectMeetings: true,
  globalMarkShortcut: true,
  voiceProfilesMe: false,
  voiceProfilesThirdParty: false,
  strictOffline: false,
  meetingLanguage: "auto",
  globalRecordShortcut: true,
  updateCheck: true,
  appLock: new URLSearchParams(location.search).has("locked"),
  lockAfterMinutes: 5,
  liveMode: "auto",
  cloudProvider: "",
  cloudModel: "",
  cloudRedact: true,
  audioRetentionDays: 0,
  consentMessageEn: "",
  consentMessageVi: "",
};
let lineSeq = 0;
/** Speakers made by a split in this session (ids after the scripted ones). */
const extraSpeakers: number[] = [];
let recoveredShown = false;
let crashAcknowledged = false;

// App updates: `?update=1` pretends a newer version is downloaded and ready.
const updateReady = new URLSearchParams(location.search).has("update");
const updateStatus = (): UpdateStatus => ({
  configured: true,
  checking: false,
  lastCheck: Date.now() - 3_600_000,
  available: updateReady ? "0.1.0-alpha.2" : null,
  notesUrl: null,
  ready: updateReady,
  runningPulled: false,
  reinstallNeeded: false,
  error: null,
});
const updateListeners = new Set<(e: UpdateChanged) => void>();

// App lock: `?locked=1` opens locked; unlocking always succeeds on the mock.
let locked = settings.appLock;
const lockListeners = new Set<(e: LockChanged) => void>();
const setLocked = (v: boolean) => {
  locked = v;
  lockListeners.forEach((l) => l({ locked: v }));
};

const commands: Commands = {
  lockState: () => ok(locked),
  lockNow: () => {
    if (settings.appLock) setLocked(true);
    return ok(locked);
  },
  unlock: () => {
    setLocked(false);
    return ok(true);
  },
  setAppLock: (on, afterMinutes) => {
    settings = { ...settings, appLock: on, lockAfterMinutes: afterMinutes };
    return ok(settings);
  },
  updateStatus: () => Promise.resolve(updateStatus()),
  checkForUpdates: () => {
    const status = updateStatus();
    updateListeners.forEach((l) => l({ status }));
    return ok(status);
  },
  // The mock can't restart itself.
  installUpdate: () => fail("updates are installed by the app, not the mock"),
  ...reviewCommands({ rows, process }),
  ...peopleCommands({ rows, voiceReady: () => voiceInstalled, recording: () => session != null }),
  ...aiCommands({
    rows,
    process,
    transcript: (m) => (rows.some((r) => r.gid === m) ? transcriptOf(m) : null),
    names: namesOf,
    strictOffline: () => settings.strictOffline,
  }),
  appVersion: () => Promise.resolve<AppVersion>({ app: "0.1.0", core: "0.1.0 (mock)" }),
  startRecording: (mode) => {
    if (session) return fail("a recording is already running");
    const id = `mock-${++meetings}`;
    session = { id, mode, startedAt: Date.now(), pausedMs: 0, pausedAt: null, timers: [], next: 0, arrived: new Set() };
    rows.unshift({
      gid: id,
      title: "",
      startedAt: Date.now(),
      durationMs: 0,
      source: "live",
      mode,
      status: "recording",
      transcriptVersion: 0,
      cloudUsed: false,
      consentConfirmed: false,
      template: null,
      people: [],
      job: null,
    });
    emit({ type: "stateChanged", meeting: id, state: "starting" });
    emit({ type: "sessionStarted", meeting: id, mode, language: null, title: "" });
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
    setRow(s.id, { durationMs });
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
  splitSpeaker: (from, lines) => {
    if (!session) return fail("nothing is recording");
    if (!lines.length) return ok(null);
    const id = Math.max(0, ...[...session.arrived].map((i) => i + 1), ...extraSpeakers) + 1;
    extraSpeakers.push(id);
    emit({
      type: "speakerSplit",
      meeting: session.id,
      from,
      speaker: { id, label: `Speaker ${id}`, colorSlot: SLOTS[(id - 1) % SLOTS.length], isMe: false, provisional: false, notPerson: false, others: false },
      lines,
    });
    return ok(id);
  },
  speakerNotAPerson: (id) => {
    if (!session) return fail("nothing is recording");
    emit({ type: "speakerNotAPerson", meeting: session.id, id });
    return ok(null);
  },
  quitApp: () => ok(null),
  listMeetings: (limit, offset) => ok(rows.slice(offset, offset + limit)),
  setMeetingTitle: (meeting, title) => {
    const r = rows.find((x) => x.gid === meeting);
    if (r) r.title = title;
    return ok(null);
  },
  noteLines: (meeting) => ok([...(notes.get(meeting) ?? [])]),
  addNoteLine: (meeting, text, tMs, kind) => {
    const line: NoteLine = { gid: `note-${++lineSeq}`, text, tMs, kind };
    notes.set(meeting, [...(notes.get(meeting) ?? []), line]);
    if (kind !== "note" && session) emit({ type: "markAdded", meeting, tMs: tMs ?? 0 });
    return ok(line);
  },
  updateNoteLine: (_meeting, gid, text) => {
    for (const ls of notes.values()) for (const l of ls) if (l.gid === gid) l.text = text;
    return ok(null);
  },
  deleteNoteLine: (_meeting, gid) => {
    for (const [m, ls] of notes) notes.set(m, ls.filter((l) => l.gid !== gid));
    return ok(null);
  },
  discardFrom: (fromMs) => {
    if (!session) return fail("nothing is recording");
    const cut = Math.max(0, fromMs ?? 0);
    emit({ type: "discardApplied", meeting: session.id, fromMs: cut });
    return ok(cut);
  },
  discardPreview: (seconds) => {
    if (!session) return fail("nothing is recording");
    const fromMs = Math.max(0, nowMs(session) - (seconds ?? 0) * 1000);
    const lines = sample.transcript.slice(0, session.next).map((l) => l.x).slice(-2);
    return ok({ fromMs, lines, notes: [], marks: 0 });
  },
  getSettings: () => ok({ ...settings, voiceProfilesMe: voiceInstalled }),
  updateSettings: (patch) => {
    const defined = Object.fromEntries(Object.entries(patch).filter(([, v]) => v != null));
    settings = { ...settings, ...defined, voiceProfilesMe: false, voiceProfilesThirdParty: false };
    return ok({ ...settings, voiceProfilesMe: voiceInstalled });
  },
  micPermission: () => Promise.resolve("granted"),
  requestMicPermission: () => ok("granted"),
  replyMeetingDetected: () => ok(null),
  // The mock serves no audio; the UI handles a sample that doesn't load.
  issueAudioSample: () => fail("no audio on the mock core"),
  showMain: () => ok(null),
  setConsentConfirmed: (meeting, confirmed) => {
    const r = rows.find((x) => x.gid === meeting);
    if (r) r.consentConfirmed = confirmed;
    return ok(null);
  },
  sessionSnapshot: () =>
    ok(
      session
        ? {
            seq: seq - 1,
            meeting: session.id,
            state: session.pausedAt != null ? "paused" : "recording",
            nowMs: nowMs(session),
            transcribing: true,
            speakers: [...session.arrived].map((i) => speaker(i, session!.mode)),
            lines: [],
            marks: [],
            mode: session.mode,
            language: null,
            title: "",
            consentConfirmed: false,
          }
        : null,
    ),
  modelsStatus: () =>
    ok({
      tier: "balanced",
      models: MODELS.map((m) => ({ ...m, installed: m.role === "voice" ? voiceInstalled : true, partialBytes: 0, damaged: false })),
      downloading: false,
    }),
  // `?recovered=1` in the URL pretends the last session crashed mid-meeting.
  takeRecoveredMeetings: () => {
    const once = new URLSearchParams(location.search).has("recovered") && !recoveredShown;
    recoveredShown = true;
    return ok(once ? [{ gid: "sample-0", title: rows[0]?.title ?? "", durationMs: 1_520_000 }] : []);
  },
  // `?crashed=1` in the URL pretends the last run crashed. No folder to open here.
  diagnosticsStatus: () =>
    Promise.resolve({ crashedLastRun: new URLSearchParams(location.search).has("crashed") && !crashAcknowledged, reports: 0 }),
  revealDiagnostics: () => ok(null),
  acknowledgeCrash: () => {
    crashAcknowledged = true;
    return Promise.resolve();
  },
  // Native windows and notifications don't exist in the browser.
  hidePopover: () => Promise.resolve(),
  setMiniCompact: () => ok(null),
  closeMini: () => Promise.resolve(),
  openMiniRecorder: () => Promise.resolve(),
  closeDetect: () => Promise.resolve(),
  showNotification: () => ok(null),
  requestQuitApp: () => {
    if (session) simulateQuitRequested();
    return Promise.resolve();
  },
  knownSpeakerNames: () => ok(["Linh", "Minh", "Sarah", "An Tran", "Jordan", "Priya"]),
  // A short scripted download, so onboarding screens can be built and tested.
  downloadModels: () => {
    MODELS.forEach((m, i) => {
      const total = m.size;
      // `?voiceslow=1`: the voice model never finishes (to see "still downloading").
      if (m.role === "voice" && new URLSearchParams(location.search).has("voiceslow")) {
        window.setTimeout(() => emitDownload({ model: m.id, phase: "downloading", done: total * 0.4, total, error: null }), 100);
        return;
      }
      [0.25, 0.6, 1].forEach((f, k) =>
        window.setTimeout(() => emitDownload({ model: m.id, phase: "downloading", done: total * f, total, error: null }), 300 * (i * 4 + k)),
      );
      window.setTimeout(() => {
        if (m.role === "voice") voiceInstalled = true;
        emitDownload({ model: m.id, phase: "done", done: total, total, error: null });
      }, 300 * (i * 4 + 3));
    });
    return ok(null);
  },
  cancelModelDownload: () => Promise.resolve(),
  retryMeeting: (meeting) => {
    const r = rows.find((x) => x.gid === meeting);
    if (r?.status === "failed") {
      r.status = "processing";
      process(meeting);
      return ok(1);
    }
    return ok(0);
  },
  deleteMeeting: (meeting) => {
    const i = rows.findIndex((r) => r.gid === meeting);
    if (i >= 0) rows.splice(i, 1);
    return ok(null);
  },
  meetingSpeakers: () =>
    ok(
      // Unnamed after processing, as "Name your speakers" expects.
      [0, 1, 2, 3].map((i) => ({
        gid: `spk-${i}`,
        name: null,
        number: i + 1,
        colorSlot: [1, 2, 4, 8][i],
        isMe: i === 0,
        notPerson: false,
        lines: 3,
        sampleT0Ms: i * 10_000,
        sampleT1Ms: i * 10_000 + 3000,
        suggestion: null,
      })),
    ),
  renameMeetingSpeaker: () => ok(null),
  hasRecoveryKey: () => ok(false),
  // A fixed sample phrase (the real one comes from the store's word list).
  createRecoveryKey: () => Promise.resolve(Array.from({ length: 24 }, (_, i) => `word${i + 1}`)),
  confirmRecoveryKey: (words) => ok(words.length === 24 && words.every((w, i) => w === `word${i + 1}`)),
  cancelRecoveryKey: () => Promise.resolve(),
  openPrivacySettings: () => ok(null),
  // Levels and one line for 3 s, then the test meeting goes away.
  testCapture: () => {
    const id = `test-${++meetings}`;
    emit({ type: "stateChanged", meeting: id, state: "starting" });
    emit({ type: "stateChanged", meeting: id, state: "recording" });
    for (let k = 0; k < 6; k++) {
      window.setTimeout(() => emit({ type: "levelMeter", meeting: id, micDbfs: -22 + k, systemDbfs: -26 + k }), 300 * k);
    }
    window.setTimeout(() => emit({ type: "transcriptPartial", meeting: id, track: 0, text: "Okay, bắt đầu nhé." }), 1200);
    window.setTimeout(() => {
      emit({ type: "stateChanged", meeting: id, state: "stopping" });
      emit({ type: "stateChanged", meeting: id, state: "ready" });
    }, 3000);
    return ok(id);
  },
};

const MODELS = [
  { id: "nemotron-3.5-asr", role: "asr", size: 1.2e9 },
  { id: "nemotron-3-diarization", role: "diarization", size: 0.2e9 },
  { id: "qwen3-4b", role: "llm", size: 2.5e9 },
  { id: "campplus-voice", role: "voice", size: 0.03e9 },
];
/** `?novoice=1`: the voice model isn't installed (a download installs it). */
let voiceInstalled = !new URLSearchParams(location.search).has("novoice");

const detectListeners = new Set<(e: MeetingDetected) => void>();
const downloadListeners = new Set<(e: ModelDownload) => void>();
const emitDownload = (e: ModelDownload) => downloadListeners.forEach((l) => l(e));

/** Dev/test hook: emit any core event (capture conditions, errors, …). */
export function simulateCoreEvent(event: Event) {
  emit(event);
}

/** Dev/test hook: the app asks "Stop and quit?". */
export function simulateQuitRequested() {
  for (const l of quitListeners) l({});
}
const navigateListeners = new Set<(e: Navigate) => void>();
const quitListeners = new Set<(e: QuitRequested) => void>();
const on =
  <T>(set: Set<(e: T) => void>) =>
  (cb: (e: T) => void) => {
    set.add(cb);
    return Promise.resolve(() => void set.delete(cb));
  };

/** Dev/test hook: pretend a meeting app started using the mic. */
export function simulateMeetingDetected(e: MeetingDetected = { app: "zoom", appName: "Zoom", browser: false }) {
  for (const l of detectListeners) l(e);
}

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
  onMeetingDetected: on(detectListeners),
  onNavigate: on(navigateListeners),
  onQuitRequested: on(quitListeners),
  onModelDownload: on(downloadListeners),
  onImportStaged,
  onImportUpdate,
  onUpdateChanged: on(updateListeners),
  onLockChanged: on(lockListeners),
  audioUrl,
};


// Playwright and the dev console drive the mock through these (mock only;
// the Tauri build never loads this module).
(window as unknown as { __ghiMock: object }).__ghiMock = {
  simulateCoreEvent,
  simulateMeetingDetected,
  simulateQuitRequested,
  simulateImportDrop,
  /** Stores a (fake) API key, as Settings → AI would. */
  setCloudKey: (provider: string) => commands.setCloudKey(provider, "test-key"),
};
