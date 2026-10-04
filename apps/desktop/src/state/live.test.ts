// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import type { CoreEvent, Event, LineInfo, SpeakerInfo } from "../bindings";
import { fromSnapshot, initialLive, reduce, type LiveState } from "./live";

let seq = 0;
const env = (event: Event): CoreEvent => ({ seq: seq++, atMs: 0, event });
const run = (events: Event[], from: LiveState = initialLive) => events.reduce((s, e) => reduce(s, env(e)), from);
const M = "m1";
const speaker = (id: number, label: string, extra: Partial<SpeakerInfo> = {}): SpeakerInfo => ({
  id,
  label,
  colorSlot: id,
  isMe: false,
  provisional: false,
  notPerson: false,
  others: false,
  ...extra,
});
const line = (speaker: number, t0: number, t1: number, text: string): LineInfo => ({
  gid: `g${t0}`,
  speaker,
  t0Ms: t0,
  t1Ms: t1,
  text,
  overlap: false,
  words: [],
});

describe("live reducer", () => {
  it("builds the session from a recorded stream", () => {
    const s = run([
      { type: "stateChanged", meeting: M, state: "starting" },
      { type: "stateChanged", meeting: M, state: "recording" },
      { type: "speakerArrived", meeting: M, speaker: speaker(1, "Speaker 1") },
      { type: "transcriptPartial", meeting: M, track: 0, text: "xin" },
      { type: "transcriptFinal", meeting: M, line: line(1, 1000, 2000, "xin chào mọi người") },
      { type: "speakerRenamed", meeting: M, speaker: speaker(1, "Linh") },
      { type: "markAdded", meeting: M, tMs: 2500 },
      { type: "health", meeting: M, asrLagS: 0.4, asrSkippedS: 0, aec: true },
    ]);
    expect(s.state).toBe("recording");
    expect(s.lines.map((l) => l.text)).toEqual(["xin chào mọi người"]);
    expect(s.partial).toEqual({});
    expect(s.speakers[1].label).toBe("Linh");
    expect(s.marks).toEqual([2500]);
    expect([s.asrLagS, s.aec]).toEqual([0.4, true]);
  });

  it("merge moves lines; discard drops what came after the cut", () => {
    const s = run([
      { type: "stateChanged", meeting: M, state: "starting" },
      { type: "speakerArrived", meeting: M, speaker: speaker(1, "Speaker 1") },
      { type: "speakerArrived", meeting: M, speaker: speaker(2, "Speaker 2") },
      { type: "transcriptFinal", meeting: M, line: line(1, 0, 1000, "a") },
      { type: "transcriptFinal", meeting: M, line: line(2, 1000, 2000, "b") },
      { type: "markAdded", meeting: M, tMs: 1500 },
      { type: "speakersMerged", meeting: M, from: 2, into: 1 },
      { type: "discardApplied", meeting: M, fromMs: 1200 },
    ]);
    expect(Object.keys(s.speakers)).toEqual(["1"]);
    expect(s.lines.map((l) => [l.text, l.speaker])).toEqual([["a", 1]]);
    expect(s.marks).toEqual([]);
  });

  it("models missing: a record-only session", () => {
    const s = run([
      { type: "stateChanged", meeting: M, state: "starting" },
      { type: "error", meeting: M, kind: "modelsMissing", message: "no speech engines" },
    ]);
    expect(s.recordOnly).toBe(true);
    expect(s.errors).toHaveLength(1);
  });

  it("a new session starts clean; other meetings' events are ignored", () => {
    const first = run([
      { type: "stateChanged", meeting: M, state: "starting" },
      { type: "transcriptFinal", meeting: M, line: line(1, 0, 1000, "a") },
      { type: "stateChanged", meeting: M, state: "processing" },
    ]);
    const s = run(
      [
        { type: "transcriptFinal", meeting: "older", line: line(1, 0, 1000, "x") },
        { type: "stateChanged", meeting: "m2", state: "starting" },
      ],
      first,
    );
    expect([s.meeting, s.state, s.lines.length]).toEqual(["m2", "starting", 0]);
  });
});

describe("recording clock", () => {
  it("excludes paused time", async () => {
    const { elapsedMs } = await import("./live");
    const at = (atMs: number, state: "starting" | "recording" | "paused"): CoreEvent => ({ seq: null, atMs, event: { type: "stateChanged", meeting: M, state } });
    let s = reduce(initialLive, at(1000, "starting"));
    s = reduce(s, at(1000, "recording"));
    s = reduce(s, at(5000, "paused"));
    expect(elapsedMs(s, 9000)).toBe(4000);
    s = reduce(s, at(8000, "recording"));
    expect(elapsedMs(s, 10_000)).toBe(6000);
  });
});

describe("capture conditions", () => {
  it("track sleep, silent system audio, disk and route", () => {
    const s = run([
      { type: "stateChanged", meeting: M, state: "starting" },
      { type: "slept", meeting: M },
      { type: "silentSystemTrack", meeting: M, silentS: 10 },
      { type: "diskLow", meeting: M, freeBytes: 2e9 },
      { type: "trackLost", meeting: M, track: 0 },
      { type: "routeChanged", meeting: M, bluetoothHfp: true },
    ]);
    expect(s.capture).toEqual({ asleep: true, systemSilent: true, diskLowBytes: 2e9, diskFull: false, lostTracks: [0], bluetoothHfp: true });
    const t = run([{ type: "woke", meeting: M }, { type: "systemAudioRestarted", meeting: M }], s);
    expect([t.capture.asleep, t.capture.systemSilent]).toEqual([false, false]);
  });
});

describe("snapshot", () => {
  it("restores a session and skips events it already has", () => {
    const snap = {
      seq: 10,
      meeting: M,
      state: "recording" as const,
      nowMs: 5000,
      transcribing: true,
      speakers: [speaker(1, "Linh")],
      lines: [line(1, 0, 1000, "a")],
      marks: [500],
      mode: "call",
      language: null,
      title: "Standup",
      consentConfirmed: true,
      sensitive: false,
    };
    let s = fromSnapshot(snap, 100_000);
    expect([s.meeting, s.lines.length, s.speakers[1].label, s.startedAtMs]).toEqual([M, 1, "Linh", 95_000]);
    // An event from before the snapshot, and a line it already has: no change.
    s = reduce(s, { seq: 9, atMs: 0, event: { type: "markAdded", meeting: M, tMs: 1 } });
    s = reduce(s, { seq: 11, atMs: 0, event: { type: "transcriptFinal", meeting: M, line: line(1, 0, 1000, "a") } });
    expect([s.marks, s.lines.length]).toEqual([[500], 1]);
    s = reduce(s, { seq: 12, atMs: 0, event: { type: "transcriptFinal", meeting: M, line: line(1, 1000, 2000, "b") } });
    expect(s.lines.length).toBe(2);
  });
});

describe("session info", () => {
  it("comes with sessionStarted", () => {
    const s = run([
      { type: "stateChanged", meeting: M, state: "starting" },
      { type: "sessionStarted", meeting: M, mode: "room", language: "vi", title: "Họp nhóm" },
      { type: "stateChanged", meeting: M, state: "recording" },
    ]);
    expect(s.session).toEqual({ mode: "room", language: "vi", title: "Họp nhóm", consentConfirmed: false, sensitive: false });
  });

  it("turns sensitive on with sensitiveChanged and restores it from a snapshot", () => {
    const s = run([
      { type: "stateChanged", meeting: M, state: "starting" },
      { type: "sessionStarted", meeting: M, mode: "room", language: null, title: "" },
      { type: "sensitiveChanged", meeting: M, sensitive: true },
    ]);
    expect(s.session?.sensitive).toBe(true);
    const snap = fromSnapshot(
      {
        seq: 3,
        meeting: M,
        state: "recording",
        nowMs: 0,
        transcribing: true,
        speakers: [],
        lines: [],
        marks: [],
        mode: "room",
        language: null,
        title: "",
        consentConfirmed: false,
        sensitive: true,
      },
      0,
    );
    expect(snap.session?.sensitive).toBe(true);
  });
});

describe("split", () => {
  it("moves the split lines to the new speaker", () => {
    const s = run([
      { type: "stateChanged", meeting: M, state: "starting" },
      { type: "speakerArrived", meeting: M, speaker: speaker(1, "Speaker 1") },
      { type: "transcriptFinal", meeting: M, line: line(1, 0, 1000, "a") },
      { type: "transcriptFinal", meeting: M, line: line(1, 1000, 2000, "b") },
      { type: "speakerSplit", meeting: M, from: 1, speaker: speaker(2, "Speaker 2"), lines: ["g1000"] },
    ]);
    expect(s.lines.map((l) => [l.text, l.speaker])).toEqual([["a", 1], ["b", 2]]);
  });
});
