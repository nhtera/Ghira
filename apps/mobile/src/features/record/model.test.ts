// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import type {
  CoreEvent,
  Event,
  LineInfo,
  MobileEvent,
  RecordState,
  SpeakerInfo,
} from "../../bindings";
import {
  catchUpPercent,
  initialModel,
  isCapturing,
  isRecording,
  markLine,
  pushLevel,
  LEVELS,
  reducer,
  speakerInitial,
  type Action,
  type RecordModel,
} from "./model";

const run = (actions: Action[], from: RecordModel = initialModel) =>
  actions.reduce(reducer, from);
const mobile = (event: MobileEvent): Action => ({ type: "mobile", event });
const core = (event: Event, seq: number | null = null): Action => ({
  type: "core",
  env: { seq, atMs: null, event } satisfies CoreEvent,
});
const speaker = (id: number, label = `Speaker ${id}`): SpeakerInfo => ({
  id,
  label,
  colorSlot: id,
  isMe: false,
  provisional: false,
  notPerson: false,
  others: false,
});
const line = (gid: string, sp: number | null, text = "xin chào"): LineInfo => ({
  gid,
  speaker: sp,
  t0Ms: 1000,
  t1Ms: 2000,
  text,
  overlap: false,
  words: [],
});
const state = (over: Partial<RecordState> = {}): RecordState => ({
  phase: "live",
  recording: true,
  elapsedS: 12,
  marks: 1,
  levelDb: -20,
  backlogS: null,
  catchUpX: null,
  pocket: false,
  live: true,
  recordOnlyReason: null,
  session: null,
  ...over,
});

describe("phase", () => {
  it("starts a fresh recording from nothing and clears the saved flag", () => {
    const done = run([mobile({ type: "phase", phase: "done" })]);
    expect(done.saved).toBe(true);
    const next = run(
      [
        core({ type: "markAdded", meeting: "m", tMs: 1 }),
        mobile({ type: "phase", phase: "loading" }),
      ],
      done,
    );
    expect(next).toMatchObject({ phase: "loading", saved: false });
  });

  it("clears the session when it ends", () => {
    const m = run([
      mobile({ type: "phase", phase: "live" }),
      core({ type: "speakerArrived", meeting: "m", speaker: speaker(1) }),
      core({ type: "transcriptFinal", meeting: "m", line: line("a", 1) }),
      mobile({ type: "phase", phase: "done" }),
    ]);
    expect(m.lines).toEqual([]);
    expect(m.phase).toBe("done");
    expect(m.saved).toBe(true);
  });

  it("keeps the interruption until another phase answers it (never auto-resumes)", () => {
    let m = run([
      mobile({ type: "phase", phase: "live" }),
      mobile({ type: "interruption", began: true, kind: "call" }),
      mobile({ type: "phase", phase: "interrupted" }),
    ]);
    expect(m.interruption).toEqual({ call: true, recordedS: 0 });
    m = run([mobile({ type: "interruption", began: false, kind: "call" })], m);
    expect(m.phase).toBe("interrupted");
    expect(m.interruption).not.toBeNull();
    m = run([mobile({ type: "phase", phase: "live" })], m);
    expect(m.interruption).toBeNull();
  });

  it("answers a resume prompt for an interrupted snapshot", () => {
    const m = run([
      { type: "snapshot", state: state({ phase: "interrupted" }) },
      {
        type: "prompt",
        prompt: { pending: true, meeting: "m", recordedS: 40, call: true },
      },
    ]);
    expect(m.interruption).toEqual({ call: true, recordedS: 40 });
    expect(
      run([
        {
          type: "prompt",
          prompt: { pending: false, meeting: "", recordedS: null, call: false },
        },
      ]).interruption,
    ).toBeNull();
  });
});

describe("clock", () => {
  it("runs only while audio is recorded", () => {
    for (const phase of [
      "live",
      "locked",
      "catchingUp",
      "hot",
      "recordOnly",
    ] as const) {
      expect(isRecording(phase)).toBe(true);
      expect(run([{ type: "tick" }], { ...initialModel, phase }).elapsedS).toBe(
        1,
      );
    }
    for (const phase of [
      "idle",
      "loading",
      "paused",
      "interrupted",
      "finishing",
      "done",
    ] as const) {
      expect(run([{ type: "tick" }], { ...initialModel, phase }).elapsedS).toBe(
        0,
      );
    }
  });
});

describe("backlog", () => {
  it("counts the catch-up percent down from the peak", () => {
    let m = run([
      mobile({ type: "phase", phase: "catchingUp" }),
      mobile({ type: "backlog", backlogS: 60, catchUpX: 2 }),
    ]);
    expect(catchUpPercent(m)).toBe(0);
    m = run([mobile({ type: "backlog", backlogS: 15, catchUpX: 2 })], m);
    expect(catchUpPercent(m)).toBe(75);
    m = run([mobile({ type: "backlog", backlogS: 0, catchUpX: null })], m);
    expect(m.backlogPeakS).toBe(0);
  });

  it("never leaves 0..100", () => {
    expect(catchUpPercent({ backlogS: 90, backlogPeakS: 60 })).toBe(0);
    expect(catchUpPercent({ backlogS: -3, backlogPeakS: 60 })).toBe(100);
    expect(catchUpPercent({ backlogS: 5, backlogPeakS: 0 })).toBe(0);
  });

  it("tracks pocket and thermal", () => {
    const m = run([
      mobile({ type: "pocket", muffled: true }),
      mobile({ type: "thermal", level: 2 }),
    ]);
    expect(m).toMatchObject({ pocket: true, thermal: 2 });
    expect(run([mobile({ type: "pocket", muffled: false })], m).pocket).toBe(
      false,
    );
  });
});

describe("transcript", () => {
  const speakers = [
    core({ type: "speakerArrived", meeting: "m", speaker: speaker(1) }),
    core({ type: "speakerArrived", meeting: "m", speaker: speaker(2) }),
  ];

  it("announces a speaker turn once, not every line", () => {
    let m = run([
      ...speakers,
      core({ type: "transcriptFinal", meeting: "m", line: line("a", 1) }),
    ]);
    expect(m.announce).toEqual({ name: "Speaker 1", n: 1 });
    m = run(
      [core({ type: "transcriptFinal", meeting: "m", line: line("b", 1) })],
      m,
    );
    expect(m.announce?.n).toBe(1);
    m = run(
      [core({ type: "transcriptFinal", meeting: "m", line: line("c", 2) })],
      m,
    );
    expect(m.announce).toEqual({ name: "Speaker 2", n: 2 });
  });

  it("ignores a line it already has and events a snapshot covered", () => {
    let m = run([
      {
        type: "snapshot",
        state: state({
          session: {
            seq: 5,
            meeting: "m",
            state: "recording",
            nowMs: null,
            transcribing: true,
            mode: "room",
            language: null,
            title: "",
            consentConfirmed: true,
            sensitive: false,
            speakers: [speaker(1)],
            lines: [line("a", 1)],
            marks: [],
          },
        }),
      },
    ]);
    expect(m.lines).toHaveLength(1);
    m = run(
      [
        core({ type: "transcriptFinal", meeting: "m", line: line("a", 1) }, 5),
        core({ type: "transcriptFinal", meeting: "m", line: line("b", 1) }, 6),
      ],
      m,
    );
    expect(m.lines.map((l) => l.key)).toEqual(["a", "b"]);
    m = run(
      [core({ type: "transcriptFinal", meeting: "m", line: line("b", 1) }, 7)],
      m,
    );
    expect(m.lines).toHaveLength(2);
  });

  it("keeps the partial until a final line replaces it", () => {
    let m = run([
      core({
        type: "transcriptPartial",
        meeting: "m",
        track: 0,
        text: "chốt scope",
      }),
    ]);
    expect(m.partial).toBe("chốt scope");
    m = run(
      [
        core({
          type: "transcriptPartial",
          meeting: "m",
          track: 1,
          text: "system",
        }),
      ],
      m,
    );
    expect(m.partial).toBe("chốt scope");
    m = run(
      [core({ type: "transcriptFinal", meeting: "m", line: line("a", null) })],
      m,
    );
    expect(m.partial).toBe("");
    expect(m.announce).toBeNull();
  });

  it("marks lines from the snapshot and counts new marks", () => {
    const m = run([
      {
        type: "snapshot",
        state: state({
          marks: 1,
          session: {
            seq: 1,
            meeting: "m",
            state: "recording",
            nowMs: null,
            transcribing: true,
            mode: "room",
            language: null,
            title: "",
            consentConfirmed: true,
            sensitive: false,
            speakers: [],
            lines: [line("a", 1), { ...line("b", 1), t0Ms: 9000, t1Ms: 9500 }],
            marks: [1500],
          },
        }),
      },
      core({ type: "markAdded", meeting: "m", tMs: 5 }, 2),
    ]);
    expect(m.lines.map((l) => l.marked)).toEqual([true, false]);
    expect(m.marks).toBe(2);
  });

  it("keeps the last 40 levels", () => {
    const levels = Array.from({ length: 50 }, (_, i) => i).reduce(pushLevel, [] as number[]);
    expect(levels).toHaveLength(LEVELS);
    expect(levels.at(-1)).toBe(49);
  });

  it("does not announce a provisional speaker", () => {
    const m = run([
      core({ type: "speakerArrived", meeting: "m", speaker: { ...speaker(1), provisional: true } }),
      core({ type: "transcriptFinal", meeting: "m", line: line("a", 1) }),
    ]);
    expect(m.announce).toBeNull();
  });

  it("stars the line a mark falls on, once, and ignores a mark the snapshot already has", () => {
    const withLines = run([
      core({ type: "transcriptFinal", meeting: "m", line: { ...line("a", 1), t0Ms: 0 } }),
      core({ type: "transcriptFinal", meeting: "m", line: { ...line("b", 1), t0Ms: 5000 } }),
    ]);
    const marked = run([core({ type: "markAdded", meeting: "m", tMs: 6000 })], withLines);
    expect(marked.lines.map((l) => l.marked)).toEqual([false, true]);
    expect(marked.marks).toBe(1);
    expect(run([core({ type: "markAdded", meeting: "m", tMs: 6000 })], marked).marks).toBe(1);
    expect(markLine(marked.lines, 100).map((l) => l.marked)).toEqual([true, true]);
    expect(markLine([], 1)).toEqual([]);
  });
});

describe("a session that ends", () => {
  it("is not restarted by a late recordOnly after done (stopped record-only)", () => {
    let m = run([mobile({ type: "phase", phase: "loading" }), mobile({ type: "phase", phase: "recordOnly" }), mobile({ type: "phase", phase: "done" })]);
    expect(m).toMatchObject({ phase: "done", saved: true, recording: false });
    m = run([mobile({ type: "phase", phase: "recordOnly" })], m);
    expect(m).toMatchObject({ phase: "done", saved: true });
    expect(isCapturing(m)).toBe(false);
    expect(run([{ type: "tick" }], m).elapsedS).toBe(0);
    // Released to idle, the Saved notice stays until dismissed; a new start works.
    m = run([mobile({ type: "phase", phase: "idle" })], m);
    expect(m).toMatchObject({ phase: "idle", saved: true });
    expect(run([mobile({ type: "phase", phase: "loading" })], m).saved).toBe(false);
  });
});

describe("loading with a session", () => {
  it("is recording: the clock runs", () => {
    const m = run([core({ type: "sessionStarted", meeting: "m", mode: "room", language: null, title: "" }), mobile({ type: "phase", phase: "loading" })]);
    expect(isCapturing(m)).toBe(true);
    expect(run([{ type: "tick" }, { type: "tick" }], m).elapsedS).toBe(2);
    // Without a session it is only getting ready.
    const bare = run([mobile({ type: "phase", phase: "loading" })]);
    expect(isCapturing(bare)).toBe(false);
  });

  it("follows the core's recording flag, also while locked and reloading", () => {
    const m = run([{ type: "snapshot", state: state({ phase: "loading", recording: true }) }]);
    expect(isCapturing(m)).toBe(true);
    expect(run([mobile({ type: "phase", phase: "paused" })], m).recording).toBe(false);
  });
});

describe("sensitive mode and discard", () => {
  const at = (gid: string, t0: number, t1: number): LineInfo => ({ ...line(gid, 1), t0Ms: t0, t1Ms: t1 });

  it("turns sensitive on with the event and restores it from a snapshot", () => {
    let m = run([core({ type: "sessionStarted", meeting: "m", mode: "room", language: null, title: "" })]);
    expect(m.sensitive).toBe(false);
    m = run([core({ type: "sensitiveChanged", meeting: "m", sensitive: true })], m);
    expect(m.sensitive).toBe(true);
    m = run(
      [
        {
          type: "snapshot",
          state: state({
            session: {
              seq: 3,
              meeting: "m",
              state: "recording",
              nowMs: null,
              transcribing: true,
              mode: "room",
              language: null,
              title: "",
              consentConfirmed: true,
              sensitive: true,
              speakers: [],
              lines: [],
              marks: [],
            },
          }),
        },
      ],
      initialModel,
    );
    expect(m.sensitive).toBe(true);
    // A new recording starts without it.
    m = run([core({ type: "sessionStarted", meeting: "n", mode: "room", language: null, title: "" })], m);
    expect(m.sensitive).toBe(false);
  });

  it("a discard removes the lines that end after the cut, the marks from it, and the partial", () => {
    let m = run([
      core({ type: "sessionStarted", meeting: "m", mode: "room", language: null, title: "" }),
      core({ type: "transcriptFinal", meeting: "m", line: at("a", 0, 3000) }),
      core({ type: "transcriptFinal", meeting: "m", line: at("b", 4000, 7000) }),
      core({ type: "transcriptFinal", meeting: "m", line: at("c", 8000, 9000) }),
      core({ type: "markAdded", meeting: "m", tMs: 2000 }),
      core({ type: "markAdded", meeting: "m", tMs: 8500 }),
      core({ type: "transcriptPartial", meeting: "m", track: 0, text: "dở dang" }),
    ]);
    expect([m.lines.length, m.marks]).toEqual([3, 2]);
    m = run([core({ type: "discardApplied", meeting: "m", fromMs: 3500 })], m);
    expect(m.lines.map((l) => l.key)).toEqual(["a"]);
    expect([m.marks, m.markTimes]).toEqual([1, [2000]]);
    expect(m.partial).toBe("");
  });
});

describe("speakerInitial", () => {
  it("uses the number of Speaker N and the first letter of a name", () => {
    expect(speakerInitial("Speaker 12")).toBe("12");
    expect(speakerInitial("Ệnh")).toBe("Ệ");
    expect(speakerInitial("")).toBe("?");
  });
});
