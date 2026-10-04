// SPDX-License-Identifier: Apache-2.0
// Scripted settings, privacy, app lock, cloud and share-inbox commands
// (slice 16-J). Models, voice enrollment and onboarding stay in mock-record.ts.
// Tests drive this through `window.__ghiSettingsMock`: counters per command
// ("the cloud sheet never calls cloudSend before the click"), the lock, the
// inbox and the failure switches. Time stands still: nothing happens by itself
// except the short scripted import of an inbox item.
import type { AppSettings, CloudSendResult, InboxItem, MobileSettings, Vocabulary } from "../bindings";
import type { Commands } from "./ipc";
import { recordCommands } from "./mock-record";

const ok = <T>(data: T) => ({ status: "ok" as const, data });
const fail = (error: string) => ({ status: "error" as const, error });

export interface GhiSettingsMock {
  /** How many times each command was called. */
  calls: Record<string, number>;
  /** The arguments of the last call of each command. */
  args: Record<string, unknown[]>;
  /** The app is locked (the gate shows). */
  locked: boolean;
  /** The app is still starting: lock_state and every content command answer "the app is starting". */
  starting: boolean;
  /** The phone has no passcode: unlock and the lock switch answer `noAuthMethod`. */
  noAuthMethod: boolean;
  /** Warnings and the retention note the next cloud previews carry. */
  warnings: string[];
  retentionNote: string;
  /** Face ID matches (false: it does not). */
  faceIdOk: boolean;
  /** Face ID prompts shown so far (unlock and turning the lock on or off). */
  faceIdPrompts: number;
  /** A recording or import is running (delete all answers "busy"). */
  busy: boolean;
  /** Which providers have a key stored. */
  keys: Record<string, boolean>;
  /** The key text the UI last sent (the real one goes to the Keychain). */
  lastKey: string;
  /** Cloud requests made today. */
  cloudRequests: number;
  /** Time of the newest logged cloud request (null: now); the visual tests pin it. */
  logAt: number | null;
  /** The custom vocabulary: the user's terms, the names learned from speakers, and the ones removed. */
  terms: string[];
  learned: string[];
  /** Vocabulary cap (the core's is 200). */
  maxTerms: number;
  /** Meeting ids whose cloud send fails (the notes stay local). */
  failSend: string[];
  /** Meeting ids with cloud AI off. */
  cloudLocked: string[];
  /** The share-extension inbox. */
  inbox: InboxItem[];
  /** The archive password of the last export, once it was accepted. */
  exportedWith: string | null;
  /** Everything was deleted. */
  wiped: boolean;
  /** Replaces the inbox and tells the UI the extension added files. */
  setInbox(items: InboxItem[]): void;
  /** The user chose to offer cloud notes (Settings -> Cloud notes). */
  offerCloud(on: boolean): void;
  /** The core locks or unlocks by itself (background delay, return): sets the state and sends the event. */
  setLocked(locked: boolean): void;
  /** Back to a fresh phone. */
  reset(): void;
}

declare global {
  interface Window {
    __ghiSettingsMock?: GhiSettingsMock;
  }
}

const freshApp = (): AppSettings => ({
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
});
const freshMobile = (): MobileSettings => ({ defaultTarget: "phone", modelsWifiOnly: true });

let app = freshApp();
let mobile = freshMobile();

function query(name: string): string | null {
  return typeof window === "undefined" ? null : new URLSearchParams(window.location.search).get(name);
}

const hooks: GhiSettingsMock = {
  calls: {},
  args: {},
  locked: query("locked") === "1",
  // `?starting=1500`: the core answers "starting" for that many ms after launch.
  starting: Number(query("starting")) > 0,
  noAuthMethod: false,
  warnings: [],
  retentionNote: "",
  faceIdOk: true,
  faceIdPrompts: 0,
  busy: false,
  keys: {},
  lastKey: "",
  cloudRequests: 0,
  logAt: null,
  terms: [],
  learned: ["Linh Trần"],
  maxTerms: 200,
  failSend: [],
  cloudLocked: [],
  inbox: [],
  exportedWith: null,
  wiped: false,
  setInbox(items) {
    hooks.inbox = items;
    window.__ghiMock?.simulateMobileEvent({ type: "inboxChanged" });
  },
  offerCloud(on) {
    app = { ...app, cloudOffered: on };
  },
  setLocked(locked) {
    hooks.locked = locked;
    announceLock(locked);
  },
  reset() {
    app = freshApp();
    mobile = freshMobile();
    meDeleted = false;
    ignored = [];
    Object.assign(hooks, { calls: {}, args: {}, locked: false, starting: false, noAuthMethod: false, warnings: [], retentionNote: "", faceIdOk: true, faceIdPrompts: 0, busy: false, keys: {}, lastKey: "", cloudRequests: 0, logAt: null, terms: [], learned: ["Linh Trần"], maxTerms: 200, failSend: [], cloudLocked: [], inbox: [], exportedWith: null, wiped: false });
  },
};
if (typeof window !== "undefined") {
  window.__ghiSettingsMock = hooks;
  if (hooks.starting) setTimeout(() => (hooks.starting = false), Number(query("starting")));
}

/** What the real core answers to a command that needs the store while it is locked or starting. */
export const gateMessage = (): string | null => (hooks.starting ? "the app is starting" : hooks.locked ? "the app is locked" : null);

// Commands that keep working while locked or starting (Core::store_even_locked and the app's own).
const OPEN = /^(lockState|lockNow|unlock|setAppLock|appVersion|lifecycleState|deviceTier|micPermission|requestMicPermission|openAppSettings|record\w*|models\w*|onboarding\w*|mobileSettings|setMobileSettings|voiceSetConsent|voiceEnroll\w*|enrollVoiceLevel)$/;

/** Makes the content commands refuse like Rust does while the app is locked or starting. */
export function gateContent(script: Partial<Commands>): Partial<Commands> {
  return Object.fromEntries(
    Object.entries(script).map(([name, fn]) => [
      name,
      OPEN.test(name) || name === "cloudModels"
        ? fn
        : (...a: unknown[]) => {
            const refused = gateMessage();
            return refused ? Promise.resolve({ status: "error", error: refused }) : (fn as (...x: unknown[]) => unknown)(...a);
          },
    ]),
  ) as Partial<Commands>;
}

/** `onLockChanged` listeners (mock.ts subscribes them). */
export const lockListeners = new Set<(locked: boolean) => void>();
const announceLock = (locked: boolean) => lockListeners.forEach((l) => l(locked));

const foldPhrase = (s: string) =>
  s
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .replace(/đ/gi, "d")
    .trim()
    .toLowerCase();

// Like ghi-core vocab::learned_terms: minus the removed names and the user's own terms, compared without case or accents.
let ignored: string[] = [];
const vocabularyOf = (): Vocabulary => ({
  terms: [...hooks.terms],
  learned: hooks.learned.filter((n) => ![...ignored, ...hooks.terms].some((x) => foldPhrase(x) === foldPhrase(n))),
  maxTerms: hooks.maxTerms,
});

const RAW_TEXT = "Nguyễn Văn An: Chúng ta chốt ngân sách 2 tỷ đồng cho quý bốn.\nLinh Trần: Mình sẽ gửi báo cáo cho anh An trước thứ Sáu.";
const REDACTED_TEXT = "<<PERSON_1>>: Chúng ta chốt ngân sách 2 tỷ đồng cho quý bốn.\n<<PERSON_2>>: Mình sẽ gửi báo cáo cho anh <<PERSON_1>> trước thứ Sáu.";
const plans = new Map<string, string>();

// Me's profile lives in mock-record.ts (enrollment); deleting it is Settings'
// job, so the delete is layered over those commands.
let meDeleted = false;

const scripted: Partial<Commands> = {
  voiceStatus: async () => {
    const r = await recordCommands.voiceStatus!();
    return r.status === "ok" && meDeleted ? ok({ ...r.data, meProfile: null }) : r;
  },
  voiceEnrollStop: async () => {
    meDeleted = false;
    return recordCommands.voiceEnrollStop!();
  },
  voiceDeleteMe: async () => {
    meDeleted = true;
    return ok(null);
  },
  mobileSettings: async () => ok(mobile),
  setMobileSettings: async (s) => {
    if (s.defaultTarget === "desktop") return fail("desktopUnavailable");
    mobile = { ...s };
    return ok(mobile);
  },
  getSettings: async () => ok(app),
  updateSettings: async (patch) => {
    const defined = Object.fromEntries(Object.entries(patch).filter(([, v]) => v != null));
    app = { ...app, ...defined } as AppSettings;
    return ok(app);
  },

  lockState: async () => (hooks.starting ? fail("the app is starting") : ok(hooks.locked)),
  lockNow: async () => {
    hooks.locked = app.appLock;
    if (hooks.locked) announceLock(true);
    return ok(hooks.locked);
  },
  unlock: async () => {
    hooks.faceIdPrompts += 1;
    if (hooks.noAuthMethod) return fail("noAuthMethod");
    if (!hooks.faceIdOk) return ok(false);
    hooks.locked = false;
    announceLock(false);
    return ok(true);
  },
  setAppLock: async (on, afterMinutes) => {
    if (on !== app.appLock) {
      hooks.faceIdPrompts += 1;
      if (hooks.noAuthMethod) return fail("noAuthMethod");
      if (!hooks.faceIdOk) return fail("notConfirmed");
    }
    app = { ...app, appLock: on, lockAfterMinutes: afterMinutes };
    return ok(app);
  },

  privacyExportAllShare: async (password) => {
    if (password.length < 8) return fail("passwordTooShort");
    hooks.exportedWith = password;
    return ok(null);
  },
  privacyDeleteAll: async (confirm) => {
    if (!["delete", "xoa"].includes(foldPhrase(confirm))) return fail("confirmation");
    if (hooks.busy) return fail("busy");
    app = freshApp();
    mobile = freshMobile();
    hooks.wiped = true;
    return ok(null);
  },

  cloudModels: async () => [
    { provider: "anthropic", model: "claude-sonnet-5-5" },
    { provider: "anthropic", model: "claude-haiku-4-5-20251001" },
    { provider: "openai", model: "gpt-5" },
  ],
  cloudKeys: async () => ok(["anthropic", "openai"].map((provider) => ({ provider, stored: Boolean(hooks.keys[provider]) }))),
  setCloudKey: async (provider, key) => {
    hooks.keys[provider] = true;
    hooks.lastKey = key;
    return ok(null);
  },
  deleteCloudKey: async (provider) => {
    hooks.keys[provider] = false;
    return ok(null);
  },
  cloudRequestLog: async (limit) =>
    ok(
      Array.from({ length: Math.min(hooks.cloudRequests, limit) }, (_, i) => ({
        meeting: "m-1",
        meetingTitle: i === 1 ? "" : "Weekly sync",
        provider: "anthropic",
        model: "claude-sonnet-5-5",
        tokensIn: 900,
        tokensOut: 300,
        at: (hooks.logAt ?? Date.now()) - i * 60_000,
      })),
    ),
  vocabulary: async () => ok(vocabularyOf()),
  setVocabulary: async (terms) => {
    const out: string[] = [];
    // Rust keeps the first 80 characters of each term.
    for (const t of terms.map((x) => Array.from(x.trim()).slice(0, 80).join("")).filter(Boolean)) if (!out.some((o) => foldPhrase(o) === foldPhrase(t))) out.push(t);
    if (out.length > hooks.maxTerms) return fail(`at most ${hooks.maxTerms} terms`);
    hooks.terms = out;
    return ok(vocabularyOf());
  },
  ignoreLearnedTerm: async (term) => {
    if (!ignored.some((i) => foldPhrase(i) === foldPhrase(term))) ignored.push(term);
    return ok(vocabularyOf());
  },
  cloudPreview: async (meeting, ask) => {
    if (!app.cloudOffered) return fail("cloudOff");
    if (hooks.cloudLocked.includes(meeting)) return fail("cloud AI is off for this meeting");
    const id = `plan-${plans.size + 1}`;
    plans.set(id, meeting);
    return ok({
      kind: "preview" as const,
      id,
      provider: ask.provider,
      model: ask.model,
      host: "api.anthropic.com",
      payload: `Rewrite these meeting notes.\n\n${ask.redact ? REDACTED_TEXT : RAW_TEXT}`,
      sha256: "9f2c4e7a",
      tokensEst: 1240,
      costEstUsd: 0.04,
      retentionNote: hooks.retentionNote,
      warnings: hooks.warnings,
      redactions: ask.redact ? [{ kind: "PERSON", count: 2 }] : [],
      excerptBefore: RAW_TEXT,
      excerptAfter: ask.redact ? REDACTED_TEXT : RAW_TEXT,
    });
  },
  cloudSend: async (id) => {
    if (!app.cloudOffered) return fail("cloudOff");
    const meeting = plans.get(id) ?? "";
    if (hooks.failSend.includes(meeting)) return ok<CloudSendResult>({ kind: "failed", reason: "timeout", leftDevice: false });
    hooks.cloudRequests += 1;
    return ok<CloudSendResult>({ kind: "notes" });
  },

  inboxList: async () => {
    if (hooks.locked) return fail("locked");
    return ok(hooks.inbox);
  },
  inboxConfirm: async (id, language, target) => {
    if (hooks.locked) return fail("locked");
    if (hooks.busy) return fail("busy");
    const item = hooks.inbox.find((i) => i.id === id);
    if (!item) return fail("notFound");
    if (item.state === "rejected") return fail(item.reason ?? "unreadable");
    if (target === "desktop") return fail("desktopUnavailable");
    hooks.inbox = hooks.inbox.map((i) => (i.id === id ? { ...i, language, target, state: "importing" as const } : i));
    window.__ghiMock?.simulateMobileEvent({ type: "inboxChanged" });
    setTimeout(() => {
      hooks.inbox = hooks.inbox.filter((i) => i.id !== id);
      window.__ghiMock?.simulateMobileEvent({ type: "inboxChanged" });
    }, 250);
    return ok(`imported-${id}`);
  },
  inboxDismiss: async (id) => {
    if (hooks.locked) return fail("locked");
    hooks.inbox = hooks.inbox.filter((i) => i.id !== id);
    return ok(null);
  },
};

export const settingsCommands: Partial<Commands> = Object.fromEntries(
  Object.entries(scripted).map(([name, fn]) => [
    name,
    (...a: unknown[]) => {
      hooks.calls[name] = (hooks.calls[name] ?? 0) + 1;
      hooks.args[name] = a;
      return (fn as (...x: unknown[]) => unknown)(...a);
    },
  ]),
) as Partial<Commands>;
