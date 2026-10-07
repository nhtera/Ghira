// SPDX-License-Identifier: Apache-2.0
// Scripted M1/M2 commands (slice 16-H): onboarding, microphone, voice, models and
// the recording session. Time stands still unless a test moves it: the session
// only changes on commands and on `window.__ghiRecord` calls, and events go
// through `window.__ghiMock` like the real shell's.
import type {
  AppSettings,
  CoreEvent,
  Event,
  LineInfo,
  MicPermission,
  MobileModelItem,
  MobileSettings,
  OnboardingState,
  OnboardingStep,
  RecordOnlyReason,
  RecordPhase,
  RecordState,
  RecordStart,
  SpeakerInfo,
  TierClass,
} from "../bindings";
import type { Commands } from "./ipc";
import { isSyncAvailable, isSyncPaired } from "./mock-sync";

const ok = <T>(data: T) => ({ status: "ok" as const, data });
const fail = (error: string) => ({ status: "error" as const, error });
const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Test hooks, beside `__ghiMock`. */
export interface GhiRecordMock {
  /** The microphone permission right now. */
  mic: MicPermission;
  /** What the system prompt answers (the first request). */
  micAnswer: Exclude<MicPermission, "notDetermined">;
  /** The tier `deviceTier` reports. */
  tier: TierClass;
  /** `modelsDownload` fails (and the items say so). */
  failModels: boolean;
  /** The models are on the phone already. */
  modelsReady: boolean;
  /** `modelsStatus` says the download waits for Wi-Fi. */
  onCellular: boolean;
  /** The notes model: `none` on a phone that cannot write notes (the default), else its state. */
  notesModel: "none" | "missing" | "ready";
  /** `recordStart` fails with this message. */
  failStart: string | null;
  /** The snapshot gives `reason` whatever the session is doing (events alone drive the phase in tests). */
  forceReason: boolean;
  /** The first speaker is the phone's owner (a voice profile matched): "Me". */
  firstIsMe: boolean;
  /** Why a record-only session has no transcript (the snapshot says). */
  reason: RecordOnlyReason;
  /** `onboardingState` fails with this message. */
  failOnboarding: string | null;
  /** pause / resume / mark / stop fail. */
  failAction: boolean;
  /** `recordStart` leaves the phase on `loading` until `release()` (the models take their time). */
  holdLoading: boolean;
  /** Ends a held `loading`. */
  release(): void;
  /** A phone call is active. */
  callActive: boolean;
  /** The speaker model is installed (voice enrollment works). */
  voiceModel: boolean;
  /** Seconds the enrollment has heard; null lets it rise with the clock (it ends at 25 s). */
  voiceSeconds: number | null;
  /** The loudness (0..1) the enrollment hears; null wobbles like a voice. */
  voiceLevel: number | null;
  /** `voiceEnrollStop` fails with this core code (tooShort, tooQuiet, micPermission…). */
  voiceStopError: string | null;
  /** `voiceEnrollStart` fails with this core code. */
  voiceStartError: string | null;
  /** Commands the UI called that have no other visible effect. */
  log: string[];
  /** The last `recordStart` the UI sent. */
  lastStart: RecordStart | null;
  /** The text the clipboard got (the webview has no real one in tests). */
  clipboard: string;
  /** Adds n final lines (Vietnamese and English, three speakers) as core events. */
  addLines(n: number): void;
  /** A partial (not yet final) line from the microphone track. */
  addPartial(text: string): void;
  /** Sets the steps already completed (`[]`: a first launch). */
  setOnboarding(steps: OnboardingStep[]): void;
  /** Starts a session without the UI (a reloaded webview finds it running). */
  seed(phase: RecordPhase, lines?: number): void;
}

declare global {
  interface Window {
    __ghiRecord?: GhiRecordMock;
  }
}

const SAMPLE = [
  "Chào mọi người, hôm nay mình chốt scope cho bản beta nhé.",
  "Ừ, mình cần thêm thời gian để kiểm tra phần đồng bộ.",
  "Let’s agree on who owns each action item before Friday.",
  "Bạn Linh phụ trách phần ngân sách, còn mình rà lại kế hoạch quý bốn.",
  "Ệ, ỗ, ẫ: những dấu chồng nhau không được bị cắt chữ.",
  "Okay, then we ship the build on Monday and review the feedback.",
];

const ORDER: OnboardingStep[] = ["languages", "micPriming", "consent", "pair", "processing", "models", "voice", "done"];

const defaults: GhiRecordMock = {
  notesModel: "none",
  mic: "notDetermined",
  failStart: null,
  reason: "deviceTier",
  forceReason: false,
  firstIsMe: false,
  failAction: false,
  failOnboarding: null,
  holdLoading: false,
  release: () => {},
  micAnswer: "granted",
  tier: "live",
  failModels: false,
  modelsReady: false,
  onCellular: false,
  callActive: false,
  voiceModel: true,
  voiceSeconds: null,
  voiceLevel: null,
  voiceStopError: null,
  voiceStartError: null,
  log: [],
  lastStart: null,
  clipboard: "",
  addLines: () => {},
  addPartial: () => {},
  setOnboarding: () => {},
  seed: () => {},
};

const hooks: GhiRecordMock = (typeof window !== "undefined" && (window.__ghiRecord ??= defaults)) || defaults;

// A returning user by default; the onboarding specs start over with `setOnboarding([])`.
let completed: OnboardingStep[] = [...ORDER];
let voiceConsent = false;
let enrolling = false;
let enrollStartedAt = 0;
let meProfile = false;
let mobileSettings: MobileSettings = { defaultTarget: "phone", modelsWifiOnly: true, desktopOfflineHours: 12 };
let appSettings = {
  onboardingDone: false,
  detectMeetings: false,
  globalMarkShortcut: false,
  globalRecordShortcut: false,
  voiceProfilesMe: true,
  voiceProfilesThirdParty: false,
  strictOffline: false,
  meetingLanguage: "auto",
  cloudProvider: "",
  cloudModel: "",
  cloudRedact: true,
  audioRetentionDays: 0,
  consentMessageEn: "",
  consentMessageVi: "",
  updateCheck: false,
  appLock: false,
  lockAfterMinutes: 0,
  liveMode: "auto",
  openAtLogin: false,
  showInMenuBar: false,
  notesLanguage: "meeting",
  detectApps: [],
  echoCancellation: true,
  appAudioOnly: false,
  // The user has not chosen to offer cloud notes yet: the core refuses cloud_preview/cloud_send ("cloudOff").
  cloudOffered: false,
} satisfies AppSettings as AppSettings;

let models: MobileModelItem[] = [
  { id: "asr", role: "asr", sizeBytes: 742_000_000, receivedBytes: 0, state: "missing" },
  { id: "diarization", role: "diarization", sizeBytes: 107_000_000, receivedBytes: 0, state: "missing" },
  { id: "voice", role: "voice", sizeBytes: 28_000_000, receivedBytes: 0, state: "missing" },
];

// The recording session.
let session: RecordState = idleState();
let lines: LineInfo[] = [];
let speakers: SpeakerInfo[] = [];
let seq = 0;
/** The session records without live transcription (below the live tier). */
let recordOnly = false;
/** Sensitive mode is on for the session (one way). */
let sensitive = false;

// A page reload keeps the session, like the core outliving the webview.
const KEY = "ghi-record-mock";
function persist() {
  try {
    sessionStorage.setItem(KEY, JSON.stringify({ session, lines, speakers, seq, recordOnly, completed, sensitive }));
  } catch {
    /* no storage: the mock forgets on reload */
  }
}
try {
  const saved = JSON.parse(sessionStorage.getItem(KEY) ?? "null");
  if (saved) ({ session, lines, speakers, seq, recordOnly, completed, sensitive } = saved);
  sensitive = Boolean(sensitive);
} catch {
  /* nothing saved */
}

function idleState(): RecordState {
  return { phase: "idle", recording: false, elapsedS: 0, marks: 0, levelDb: null, backlogS: null, catchUpX: null, pocket: false, live: false, recordOnlyReason: null, session: null };
}

function mobile(e: Parameters<NonNullable<Window["__ghiMock"]>["simulateMobileEvent"]>[0]) {
  window.__ghiMock?.simulateMobileEvent(e);
}

function core(event: Event) {
  seq += 1;
  const env: CoreEvent = { seq, atMs: null, event };
  persist();
  window.__ghiMock?.simulateCoreEvent(env);
}

function setPhase(phase: RecordPhase) {
  session = { ...session, phase, recording: ["live", "locked", "catchingUp", "hot", "recordOnly"].includes(phase) || (phase === "loading" && session.recording) };
  persist();
  mobile({ type: "phase", phase });
}

function speaker(id: number): SpeakerInfo {
  const me = id === 1 && hooks.firstIsMe;
  return { id, label: me ? "Me" : `Speaker ${id}`, colorSlot: id, isMe: me, provisional: false, notPerson: false, others: false };
}

function addLines(n: number) {
  for (let i = 0; i < n; i += 1) {
    const id = (lines.length % 3) + 1;
    if (!speakers.some((s) => s.id === id)) {
      const s = speaker(id);
      speakers = [...speakers, s];
      core({ type: "speakerArrived", meeting: "m-1", speaker: s });
    }
    const t0 = lines.length * 4000;
    const line: LineInfo = { gid: `l-${lines.length + 1}`, speaker: id, t0Ms: t0, t1Ms: t0 + 3500, text: SAMPLE[lines.length % SAMPLE.length], overlap: false, words: [] };
    lines = [...lines, line];
    core({ type: "transcriptFinal", track: 0, meeting: "m-1", line });
  }
}

function snapshot(): RecordState {
  if (session.phase === "idle" || session.phase === "done") return session;
  return {
    ...session,
    live: !recordOnly,
    recordOnlyReason: recordOnly || session.phase === "recordOnly" || hooks.forceReason ? hooks.reason : null,
    session: {
      seq,
      meeting: "m-1",
      state: session.phase === "paused" ? "paused" : "recording",
      nowMs: null,
      transcribing: !recordOnly,
      mode: "room",
      language: "auto",
      title: "",
      consentConfirmed: true,
      sensitive,
      speakers,
      lines,
      marks: Array.from({ length: session.marks }, (_, i) => i * 1000),
    },
  };
}

hooks.setOnboarding = (steps) => {
  completed = steps;
  persist();
};
hooks.addLines = addLines;
hooks.addPartial = (text) => core({ type: "transcriptPartial", meeting: "m-1", track: 0, text });
hooks.release = () => {
  if (session.phase === "loading") setPhase(recordOnly ? "recordOnly" : "live");
};
hooks.seed = (phase, n = 0) => {
  session = { ...idleState(), phase, live: true, recording: ["live", "locked", "catchingUp", "hot", "recordOnly"].includes(phase) };
  lines = [];
  speakers = [];
  if (n) addLines(n);
  persist();
};

const next = (step: OnboardingStep): OnboardingState => {
  if (!completed.includes(step)) completed = [...completed, step].sort((a, b) => ORDER.indexOf(a) - ORDER.indexOf(b));
  persist();
  return { completed, syncAvailable: isSyncAvailable() };
};

function emitModel(item: MobileModelItem) {
  models = models.map((m) => (m.id === item.id ? item : m));
  mobile({ type: "modelDownload", item });
}

const NOTES_MODEL: MobileModelItem = { id: "qwen3-4b", role: "notes", sizeBytes: 2_497_280_256, receivedBytes: 0, state: "missing" };
const notesModelItem = (): MobileModelItem | null =>
  hooks.notesModel === "none"
    ? null
    : hooks.notesModel === "ready"
      ? { ...NOTES_MODEL, receivedBytes: NOTES_MODEL.sizeBytes, state: "ready" }
      : hooks.onCellular
        ? { ...NOTES_MODEL, state: "waitingForWifi" }
        : NOTES_MODEL;

export const recordCommands: Partial<Commands> = {
  // First launch
  onboardingState: async () => (hooks.failOnboarding ? fail(hooks.failOnboarding) : ok({ completed, syncAvailable: isSyncAvailable() })),
  onboardingCompleteStep: async (step) => ok(next(step)),
  micPermission: async () => hooks.mic,
  requestMicPermission: async () => {
    hooks.log.push("requestMicPermission");
    if (hooks.mic === "notDetermined") hooks.mic = hooks.micAnswer;
    return hooks.mic;
  },
  openAppSettings: async () => {
    hooks.log.push("openAppSettings");
  },
  voiceStatus: async () => ok({ modelReady: hooks.voiceModel, meProfile: meProfile ? { atMs: null, samples: 3 } : null, enrolling }),
  voiceSetConsent: async (given) => {
    voiceConsent = given;
    if (!given) enrolling = false;
    return ok(null);
  },
  voiceEnrollStart: async () => {
    if (!voiceConsent) return fail("invalidConsent");
    if (!hooks.voiceModel) return fail("noModel");
    if (hooks.voiceStartError) return fail(hooks.voiceStartError);
    enrolling = true;
    enrollStartedAt = Date.now();
    return ok(null);
  },
  enrollVoiceLevel: async () => {
    if (!enrolling) return fail("notEnrolling");
    const max = 25;
    const seconds = Math.min(max, hooks.voiceSeconds ?? (Date.now() - enrollStartedAt) / 1000);
    // A voice-like wobble that rises with the read.
    const level = hooks.voiceLevel ?? Math.min(1, 0.25 + 0.5 * Math.abs(Math.sin(seconds * 3)));
    return ok({ level, seconds, maxSeconds: max, done: seconds >= max });
  },
  voiceEnrollStop: async () => {
    if (!voiceConsent) {
      enrolling = false;
      return fail("invalidConsent");
    }
    if (!enrolling) return fail("notEnrolling");
    enrolling = false;
    if (hooks.voiceStopError) return fail(hooks.voiceStopError);
    meProfile = true;
    return ok(null);
  },
  voiceEnrollCancel: async () => {
    enrolling = false;
    return ok(null);
  },
  mobileSettings: async () => ok(mobileSettings),
  setMobileSettings: async (settings) => {
    mobileSettings = settings;
    return ok(settings);
  },
  getSettings: async () => ok(appSettings),
  updateSettings: async (patch) => {
    appSettings = { ...appSettings, ...Object.fromEntries(Object.entries(patch).filter(([, v]) => v != null)) } as AppSettings;
    return ok(appSettings);
  },
  deviceTier: async () => ok({ modelId: "iPhone16,1", ramGb: 8, simulator: true, tier: hooks.tier, notes: hooks.tier === "live" }),
  modelsStatus: async () => {
    const items = models.map((m): MobileModelItem => {
      if (hooks.modelsReady) return { ...m, state: "ready" };
      return hooks.onCellular && m.state === "missing" ? { ...m, state: "waitingForWifi" } : m;
    });
    const missing = items.filter((m) => m.state !== "ready").reduce((sum, m) => sum + (m.sizeBytes ?? 0), 0);
    return ok({ items, missingBytes: missing, wifiOnly: mobileSettings.modelsWifiOnly });
  },
  modelsDownload: async (wifiOnly) => {
    hooks.log.push(`modelsDownload:${wifiOnly}`);
    if (hooks.failModels) {
      const first = models[0];
      emitModel({ ...first, state: "failed" });
      return fail("network");
    }
    for (const step of [0.4, 1]) {
      await pause(30);
      for (const m of models.filter((x) => x.state !== "ready")) {
        const done = step === 1;
        emitModel({ ...m, receivedBytes: Math.round((m.sizeBytes ?? 0) * step), state: done ? "ready" : "downloading" });
      }
    }
    return ok(null);
  },
  modelsCancel: async () => ok(null),
  notesModelStatus: async () => ok(notesModelItem()),
  modelsDownloadNotes: async (wifiOnly) => {
    hooks.log.push(`modelsDownloadNotes:${wifiOnly}`);
    if (hooks.notesModel === "none") return fail("this phone cannot write notes itself");
    for (const step of [0.4, 1]) {
      await pause(30);
      const done = step === 1;
      if (done) hooks.notesModel = "ready";
      mobile({
        type: "modelDownload",
        item: { ...NOTES_MODEL, receivedBytes: Math.round((NOTES_MODEL.sizeBytes ?? 0) * step), state: done ? "ready" : "downloading" },
      });
    }
    return ok(null);
  },
  modelsRemoveNotes: async () => {
    hooks.log.push("modelsRemoveNotes");
    if (hooks.notesModel !== "none") hooks.notesModel = "missing";
    return ok(null);
  },

  // Recording
  recordConsentMessage: async (language) => {
    const en = "Heads up: I’m recording this meeting for notes.";
    const vi = "Mình báo trước: mình đang ghi âm cuộc họp này để lấy ghi chú.";
    return ok({ en, vi, text: language === "vi" ? vi : language === "en" ? en : `${en}\n${vi}` });
  },
  recordCallActive: async () => ok(hooks.callActive),
  recordSnapshot: async () => ok(snapshot()),
  recordResumePrompt: async () => ok({ pending: session.phase === "interrupted", meeting: "m-1", recordedS: session.elapsedS, call: hooks.callActive }),
  recordStart: async (start) => {
    hooks.lastStart = start;
    hooks.log.push("recordStart");
    if (hooks.failStart) return fail(hooks.failStart);
    if (hooks.mic === "denied") return fail("microphoneDenied");
    if (enrolling) return fail("micInUse");
    if (!start.consentAcknowledged) return fail("consent");
    if (hooks.callActive && !start.callAcknowledged) return fail("callActive");
    if (start.target === "desktop" && !isSyncPaired()) return fail("pairingNotAvailable");
    if (session.phase !== "idle" && session.phase !== "done") return fail("alreadyRecording");
    // Below the live tier it always records, without a transcript.
    recordOnly = hooks.tier === "recordOnly";
    // Sensitive mode keeps nothing but the live transcript.
    if (start.sensitive && recordOnly) return fail("sensitiveNeedsTranscript");
    sensitive = start.sensitive;
    lines = [];
    speakers = [];
    session = { ...idleState(), phase: "loading", recording: true };
    core({ type: "sessionStarted", meeting: "m-1", mode: "room", language: null, title: "" });
    if (sensitive) core({ type: "sensitiveChanged", meeting: "m-1", sensitive: true });
    mobile({ type: "phase", phase: "loading" });
    if (!hooks.holdLoading) {
      await pause(20);
      setPhase(recordOnly ? "recordOnly" : "live");
    }
    return ok("m-1");
  },
  recordPause: async () => {
    hooks.log.push("recordPause");
    if (hooks.failAction) return fail("failed");
    setPhase("paused");
    return ok(null);
  },
  recordResume: async () => {
    hooks.log.push("recordResume");
    if (hooks.failAction) return fail("failed");
    setPhase(recordOnly ? "recordOnly" : "live");
    return ok(null);
  },
  recordMark: async () => {
    hooks.log.push("recordMark");
    if (hooks.failAction) return fail("failed");
    session = { ...session, marks: session.marks + 1 };
    core({ type: "markAdded", meeting: "m-1", tMs: session.marks * 1000 });
    return ok(null);
  },
  recordSetSensitive: async (on) => {
    hooks.log.push(`recordSetSensitive:${on}`);
    if (hooks.failAction) return fail("failed");
    if (!on) return sensitive ? fail("sensitive mode cannot be turned off during the recording") : ok(null);
    if (recordOnly) return fail("sensitiveNeedsTranscript");
    if (!sensitive) {
      sensitive = true;
      core({ type: "sensitiveChanged", meeting: "m-1", sensitive: true });
    }
    return ok(null);
  },
  // The last `seconds` of the recording: what the lines (4 s each) and marks (1 s apart) say.
  recordDiscardPreview: async (secondsOrNull) => {
    const seconds = secondsOrNull ?? 0;
    hooks.log.push(`recordDiscardPreview:${seconds}`);
    if (hooks.failAction) return fail("failed");
    const now = Math.max(lines.length * 4000, session.marks * 1000);
    const from = Math.max(0, now - seconds * 1000);
    return ok({ fromMs: from, lines: lines.filter((l) => (l.t1Ms ?? 0) > from).map((l) => l.text), notes: [], marks: Math.max(0, session.marks - Math.ceil(from / 1000)) });
  },
  recordDiscardFrom: async (fromMsOrNull) => {
    const fromMs = fromMsOrNull ?? 0;
    hooks.log.push(`recordDiscardFrom:${fromMs}`);
    if (hooks.failAction) return fail("failed");
    lines = lines.filter((l) => (l.t1Ms ?? 0) <= fromMs);
    session = { ...session, marks: Math.min(session.marks, Math.ceil(fromMs / 1000)) };
    core({ type: "discardApplied", meeting: "m-1", fromMs });
    return ok(fromMs);
  },
  recordStop: async () => {
    hooks.log.push("recordStop");
    if (hooks.failAction) return fail("failed");
    setPhase("finishing");
    await pause(20);
    // Done, then released to idle; never recordOnly again.
    session = { ...idleState(), phase: "done" };
    persist();
    mobile({ type: "phase", phase: "done" });
    await pause(20);
    session = idleState();
    persist();
    mobile({ type: "phase", phase: "idle" });
    return ok(null);
  },
};
