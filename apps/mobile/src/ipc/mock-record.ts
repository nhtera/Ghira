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
  /** `recordStart` fails with this message. */
  failStart: string | null;
  /** The snapshot gives `reason` whatever the session is doing (events alone drive the phase in tests). */
  forceReason: boolean;
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

const ORDER: OnboardingStep[] = ["languages", "micPriming", "consent", "processing", "models", "voice", "done"];

const defaults: GhiRecordMock = {
  mic: "notDetermined",
  failStart: null,
  reason: "deviceTier",
  forceReason: false,
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
let meProfile = false;
let mobileSettings: MobileSettings = { defaultTarget: "phone", modelsWifiOnly: true };
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

// A page reload keeps the session, like the core outliving the webview.
const KEY = "ghi-record-mock";
function persist() {
  try {
    sessionStorage.setItem(KEY, JSON.stringify({ session, lines, speakers, seq, recordOnly, completed }));
  } catch {
    /* no storage: the mock forgets on reload */
  }
}
try {
  const saved = JSON.parse(sessionStorage.getItem(KEY) ?? "null");
  if (saved) ({ session, lines, speakers, seq, recordOnly, completed } = saved);
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
  return { id, label: `Speaker ${id}`, colorSlot: id, isMe: false, provisional: false, notPerson: false, others: false };
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
    core({ type: "transcriptFinal", meeting: "m-1", line });
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
  return { completed, syncAvailable: false };
};

function emitModel(item: MobileModelItem) {
  models = models.map((m) => (m.id === item.id ? item : m));
  mobile({ type: "modelDownload", item });
}

export const recordCommands: Partial<Commands> = {
  // First launch
  onboardingState: async () => (hooks.failOnboarding ? fail(hooks.failOnboarding) : ok({ completed, syncAvailable: false })),
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
    if (!voiceConsent) return fail("consent");
    if (!hooks.voiceModel) return fail("model");
    enrolling = true;
    return ok(null);
  },
  voiceEnrollStop: async () => {
    enrolling = false;
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
  deviceTier: async () => ok({ modelId: "iPhone16,1", ramGb: 8, simulator: true, tier: hooks.tier }),
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
    if (start.target === "desktop") return fail("pairingNotAvailable");
    if (session.phase !== "idle" && session.phase !== "done") return fail("alreadyRecording");
    // Below the live tier it always records, without a transcript.
    recordOnly = hooks.tier === "recordOnly";
    lines = [];
    speakers = [];
    session = { ...idleState(), phase: "loading", recording: true };
    core({ type: "sessionStarted", meeting: "m-1", mode: "room", language: null, title: "" });
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
