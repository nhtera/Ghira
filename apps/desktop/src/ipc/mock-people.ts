// SPDX-License-Identifier: Apache-2.0
// The mock core's people side (phase 14c): People with Me and three others
// (one with a voice profile), Me enrollment with a scripted level, and the
// per-meeting voice suggestions. URL flags for the error paths:
//   ?novoice=1            the voice model isn't installed (mock.ts; modelReady false until downloaded)
//   ?voiceslow=1          its download never finishes (mock.ts)
//   ?peoplefail=<code>    listPeople fails with that code
//   ?peopleerr=<code>     merge / delete voice / remove name fail with that code
//   ?enrollfail=<code>    enrollment fails: noModel, micPermission, noMic (start); tooShort, tooQuiet (finish)
//   ?enrollspeed=<n>      the scripted reading runs n times faster (default 1)
//   ?peopleempty=1        no meetings, nobody named yet
import type { EnrollLevel, MeetingRow, PeopleList, PersonAction, PersonDetail, PersonRow, VoiceSample, VoiceStatus } from "../bindings";
import type { Commands } from "./ipc";
import { speakersOf } from "./mock-review";

type Result<T> = { status: "ok"; data: T } | { status: "error"; error: string };
const ok = <T>(data: T): Promise<Result<T>> => Promise.resolve({ status: "ok", data });
const fail = <T>(error: string): Promise<Result<T>> => Promise.resolve({ status: "error", error });
const flag = (name: string) => new URLSearchParams(location.search).get(name);

export interface PeopleHost {
  rows: MeetingRow[];
  /** The speaker model is installed. */
  voiceReady(): boolean;
  /** A recording is running (People and enrollment commands refuse then). */
  recording(): boolean;
}

type Seed = { gid: string; name: string; isMe: boolean; colorSlot: number; voiceAt: number | null; self?: boolean };
const DAY = 86_400_000;

const MAX_SECONDS = 25;
const MIN_SECONDS = 10;

type PeopleCommands = Pick<
  Commands,
  | "listPeople"
  | "personDetail"
  | "mergePeople"
  | "deleteVoiceData"
  | "removePersonName"
  | "voiceStatus"
  | "enrollVoiceStart"
  | "enrollVoiceLevel"
  | "enrollVoiceFinish"
  | "enrollVoiceCancel"
  | "setSpeakerMe"
  | "clearSpeakerMe"
  | "acceptVoiceSuggestion"
  | "dismissVoiceSuggestion"
  | "saveVoiceProfile"
>;

export function peopleCommands(host: PeopleHost): PeopleCommands {
  let people: Seed[] = [
    { gid: "person-me", name: "", isMe: true, colorSlot: 1, voiceAt: Date.now() - 30 * DAY, self: true },
    { gid: "person-linh", name: "Linh", isMe: false, colorSlot: 2, voiceAt: null },
    { gid: "person-minh", name: "Minh", isMe: false, colorSlot: 4, voiceAt: null },
    { gid: "person-sarah", name: "Sarah", isMe: false, colorSlot: 8, voiceAt: null },
  ];
  /** Names removed from notes (the person row stays only while a voice profile does). */
  const removed = new Set<string>();
  let meSamples = 6;
  let enroll: { t0: number } | null = null;

  const ready = () => host.rows.filter((r) => r.status === "ready" || r.status === "done");
  const meetingsOf = (p: Seed) => (flag("peopleempty") ? [] : p.isMe ? ready() : removed.has(p.gid) ? [] : ready().filter((r) => r.people.some((x) => x.name === p.name)));
  const actionsOf = (p: Seed): PersonAction[] => {
    const m = meetingsOf(p)[0];
    if (!m || p.isMe || removed.has(p.gid)) return [];
    return [{ gid: `act-${p.gid}`, meetingGid: m.gid, meetingTitle: m.title, text: `Follow up with ${p.name}`, due: null, dueText: "Fri" }];
  };
  const rowOf = (p: Seed): PersonRow => {
    const ms = meetingsOf(p);
    return {
      gid: p.gid,
      name: p.name,
      isMe: p.isMe,
      colorSlot: p.colorSlot,
      meetings: ms.length,
      openActions: actionsOf(p).length,
      lastMetMs: ms.reduce<number | null>((n, r) => Math.max(n ?? 0, r.startedAt ?? 0) || n, null),
      voice: p.voiceAt != null ? { kind: p.self ? "self" : "agreed", atMs: p.voiceAt } : { kind: "none", atMs: null },
    };
  };
  const find = (gid: string) => people.find((p) => p.gid === gid);
  const refusal = () => flag("peopleerr");
  const speaker = (meeting: string, gid: string) => (host.rows.some((r) => r.gid === meeting) ? speakersOf(meeting).find((s) => s.gid === gid) : undefined);
  /** In a call only the mic speaker (spk-0 here) can be Me. */
  const farSide = (meeting: string, gid: string) => host.rows.find((r) => r.gid === meeting)?.mode === "call" && gid !== "spk-0";
  const setMe = (meeting: string, gid: string) => {
    const s = speaker(meeting, gid);
    if (!s) return fail<null>("notASpeaker");
    if (farSide(meeting, gid)) return fail<null>("farSide");
    for (const o of speakersOf(meeting)) o.isMe = o.gid === gid;
    s.suggestion = null;
    return ok(null);
  };

  return {
    listPeople: () => {
      const code = flag("peoplefail");
      if (code) return fail(code);
      const list: PeopleList = { people: flag("peopleempty") ? people.filter((p) => p.isMe).map(rowOf) : people.filter((p) => p.isMe || !removed.has(p.gid) || p.voiceAt != null).map(rowOf), thirdParty: false };
      list.people.sort((a, b) => Number(b.isMe) - Number(a.isMe) || (b.lastMetMs ?? 0) - (a.lastMetMs ?? 0));
      return ok(list);
    },
    personDetail: (gid) => {
      const p = find(gid);
      if (!p) return fail("notFound");
      const ms = meetingsOf(p);
      // Other people's voice data exists only with `thirdParty`, which is off in this build.
      const samples: VoiceSample[] = p.voiceAt == null || !p.isMe ? [] : ms.slice(0, 3).map((r, i) => ({ meetingGid: r.gid, meetingTitle: r.title, t0Ms: 12_000 + i * 20_000, t1Ms: 15_000 + i * 20_000, track: p.isMe ? 0 : null }));
      const detail: PersonDetail = {
        person: rowOf(p),
        meetings: ms.slice(0, 50).map((r) => ({ gid: r.gid, title: r.title, startedAt: r.startedAt, durationMs: r.durationMs })),
        openActions: actionsOf(p),
        samples,
      };
      return ok(detail);
    },
    mergePeople: (from, into) => {
      if (refusal()) return fail(refusal()!);
      if (host.recording()) return fail("busyRecording");
      const a = find(from);
      const b = find(into);
      if (!a || !b) return fail("notFound");
      if (a.isMe || b.isMe) return fail("isMe");
      if (a.gid === b.gid) return fail("samePerson");
      // Merging voice profiles is third-party voice data: off in this build.
      if (a.voiceAt != null || b.voiceAt != null) return fail("thirdPartyOff");
      people = people.filter((p) => p.gid !== from);
      return ok(null);
    },
    deleteVoiceData: (gid) => {
      if (refusal()) return fail(refusal()!);
      const p = find(gid);
      if (!p) return fail("notFound");
      if (p.voiceAt == null) return fail("noProfile");
      p.voiceAt = null;
      if (p.isMe) meSamples = 0;
      return ok(null);
    },
    removePersonName: (gid) => {
      if (refusal()) return fail(refusal()!);
      if (host.recording()) return fail("busyRecording");
      const p = find(gid);
      if (!p) return fail("notFound");
      if (p.isMe) return fail("isMe");
      const n = meetingsOf(p).length;
      removed.add(gid);
      return ok(n);
    },
    voiceStatus: () => {
      const me = people[0]!;
      const status: VoiceStatus = {
        modelReady: host.voiceReady(),
        meProfile: me.voiceAt != null ? { atMs: me.voiceAt, samples: meSamples } : null,
        enrolling: enroll != null,
      };
      return ok(status);
    },
    enrollVoiceStart: () => {
      if (host.recording()) return fail("busyRecording");
      if (!host.voiceReady()) return fail("noModel");
      const code = flag("enrollfail");
      if (code && ["noModel", "micPermission", "noMic", "busyRecording"].includes(code)) return fail(code);
      enroll = { t0: Date.now() };
      return ok(null);
    },
    enrollVoiceLevel: () => {
      if (!enroll) return Promise.resolve({ status: "error", error: "notEnrolling" });
      const speed = Number(flag("enrollspeed")) || 1;
      const seconds = Math.min(MAX_SECONDS, ((Date.now() - enroll.t0) / 1000) * speed);
      const level: EnrollLevel = { level: 0.15 + 0.3 * Math.abs(Math.sin(seconds * 3)), seconds, maxSeconds: MAX_SECONDS, done: seconds >= MAX_SECONDS };
      return ok(level);
    },
    enrollVoiceFinish: (consentTextKey) => {
      const run = enroll;
      if (!run) return fail("notEnrolling");
      // Only the two shown sentences are accepted as the consent record.
      if (consentTextKey !== "onboarding.voice.consent_mac" && consentTextKey !== "onboarding.voice.consent_win") return fail("invalidConsent");
      const code = flag("enrollfail");
      if (code === "tooQuiet" || code === "tooShort") {
        enroll = null;
        return fail(code);
      }
      const speed = Number(flag("enrollspeed")) || 1;
      const seconds = ((Date.now() - run.t0) / 1000) * speed;
      enroll = null;
      if (seconds < MIN_SECONDS) return fail("tooShort");
      const me = people[0]!;
      me.voiceAt = Date.now();
      me.self = true;
      meSamples = 6;
      return ok(null);
    },
    enrollVoiceCancel: () => {
      enroll = null;
      return ok(null);
    },
    setSpeakerMe: (meeting, gid) => setMe(meeting, gid),
    clearSpeakerMe: (meeting, gid) => {
      const s = speaker(meeting, gid);
      if (!s) return fail("notASpeaker");
      if (!s.isMe) return fail("notMe");
      if (farSide(meeting, gid)) return fail("farSide");
      s.isMe = false;
      return ok(null);
    },
    acceptVoiceSuggestion: (meeting, gid) => {
      const s = speaker(meeting, gid);
      if (!s) return fail("notASpeaker");
      if (!s.suggestion) return fail("noSuggestion");
      return s.suggestion.isMe ? setMe(meeting, gid) : fail("thirdPartyOff");
    },
    dismissVoiceSuggestion: (meeting, gid) => {
      const s = speaker(meeting, gid);
      if (!s) return fail("notASpeaker");
      s.suggestion = null;
      return ok(null);
    },
    saveVoiceProfile: () => fail("thirdPartyOff"),
  };
}
