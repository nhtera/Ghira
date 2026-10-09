// SPDX-License-Identifier: Apache-2.0
// Scripted library, meeting and search commands (M3, M4, search; slice 16-I).
// `?meetings=empty|many` picks the dataset (default: six meetings, one of each
// state, dated so screenshots stay stable; many: 1,200 plain ones). Search mirrors ghi-store's folding (accents and
// case ignored, đ = d); the real folding is tested in ghi-store.
import type {
  ActionItemView,
  Citation,
  MeetingChip,
  MeetingDetail,
  MeetingNotes,
  MeetingRow,
  MeetingSpeaker,
  MeetingTranscript,
  NoteBlockView,
  SearchHitView,
  SegmentView,
} from "../bindings";
import type { Commands } from "./ipc";

const ok = <T>(data: T) => ({ status: "ok" as const, data });
const fail = (error: string) => ({ status: "error" as const, error });

/** Accents, case and đ ignored; same length as the input for precomposed text, so highlights index the original. */
export function fold(s: string): string {
  return Array.from(s)
    .map((c) =>
      c
        .normalize("NFD")
        .replace(/\p{M}/gu, "")
        .replace(/[đĐ]/g, "d")
        .toLowerCase(),
    )
    .join("");
}

type MockMeeting = {
  row: MeetingRow;
  detail: MeetingDetail;
  notes: MeetingNotes;
  transcript: MeetingTranscript;
  chip: MeetingChip;
};

const MIN = 60_000;
const startOfToday = () => {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  return d.getTime();
};
const DAY = 24 * 60 * MIN;

const speaker = (
  gid: string,
  number: number,
  name: string | null,
  colorSlot: number,
  isMe = false,
  lines = 0,
): MeetingSpeaker => ({
  gid,
  name,
  number,
  colorSlot,
  isMe,
  notPerson: false,
  lines,
  sampleT0Ms: null,
  sampleT1Ms: null,
  suggestion: null,
});

/** One segment; its words are spread evenly over [t0, t1). */
function segment(
  gid: string,
  speakerGid: string | null,
  t0Ms: number,
  t1Ms: number,
  text: string,
  extra: Partial<SegmentView> = {},
): SegmentView {
  const words = text.split(/\s+/).filter(Boolean);
  const step = (t1Ms - t0Ms) / Math.max(words.length, 1);
  return {
    gid,
    speakerGid,
    t0Ms,
    t1Ms,
    text,
    language: null,
    confidence: 0.9,
    edited: false,
    overlap: false,
    words: words.map((_, i) => ({
      t0Ms: Math.round(t0Ms + i * step),
      t1Ms: Math.round(t0Ms + (i + 1) * step),
      confidence: i === 3 && gid === "s-lowconf" ? 0.3 : 0.95,
    })),
    ...extra,
  };
}

const cite = (
  t0Ms: number | null,
  quote: string,
  extra: Partial<Citation> = {},
): Citation => ({
  t0Ms,
  t1Ms: t0Ms === null ? null : t0Ms + 6_000,
  quote,
  speakerGid: null,
  stale: false,
  missing: false,
  ...extra,
});

function build(
  id: string,
  title: string,
  startedAt: number,
  durationMin: number,
  chip: MeetingChip,
  o: Partial<MockMeeting> & {
    people?: [string, number][];
    summary?: string | null;
    speakers?: MeetingSpeaker[];
    job?: MeetingRow["job"];
    status?: string;
  } = {},
): MockMeeting {
  const speakers = o.speakers ?? [];
  const status = o.status ?? "ready";
  const row: MeetingRow = {
    gid: id,
    title,
    startedAt,
    durationMs: durationMin * MIN,
    source: "live",
    mode: "room",
    status,
    transcriptVersion: 2,
    cloudUsed: false,
    consentConfirmed: true,
    sensitive: false,
    template: null,
    people: (o.people ?? []).map(([name, colorSlot]) => ({ name, colorSlot, isMe: false })),
    job: o.job ?? null,
    folder: null,
    tags: [],
    sourceApp: null,
    unnamedVoices: 0,
    summary: o.summary ?? null,
  };
  return {
    row,
    chip,
    detail: {
      gid: id,
      title,
      startedAt,
      durationMs: durationMin * MIN,
      source: "live",
      mode: "room",
      language: "mixed",
      template: null,
      status,
      cloudLocked: false,
      sensitive: false,
      cloudUsed: false,
      consentConfirmed: true,
      transcriptVersion: 2,
      notesModel: null,
      sourceApp: null,
      audioAvailable: true,
      speakers,
      job: o.job ?? null,
    },
    notes: o.notes ?? { blocks: [], actionItems: [], sections: [], marks: [] },
    transcript: o.transcript ?? {
      version: 2,
      segments: [],
      marks: [],
      topics: [],
    },
  };
}

function defaults(): MockMeeting[] {
  const today = startOfToday();
  const spk = [
    speaker("sp-linh", 1, "Linh", 1, false, 3),
    speaker("sp-minh", 2, "Minh", 2, false, 2),
    speaker("sp-me", 3, "Me", 3, true, 1),
  ];
  const planning = build(
    "m-nonotes",
    "Họp kế hoạch quý 4 · Đà Nẵng",
    new Date(2026, 8, 29, 9, 30).getTime(),
    42,
    { kind: "processedOnPhone" },
    {
      people: [
        ["Linh", 1],
        ["Minh", 2],
      ],
      speakers: spk,
      transcript: {
        version: 2,
        marks: [],
        topics: [],
        segments: [
          segment(
            "s1",
            "sp-linh",
            4_000,
            12_000,
            "Chào mọi người, hôm nay mình chốt kế hoạch cho dự án Đà Nẵng.",
          ),
          segment(
            "s2",
            "sp-minh",
            12_500,
            21_000,
            "Ngân sách dự kiến khoảng năm trăm triệu đồng, chưa tính chi phí đi lại.",
          ),
          segment(
            "s-lowconf",
            "sp-me",
            21_500,
            28_000,
            "Mình nghĩ cần thêm một người phụ trách phần vận hành.",
            { overlap: true },
          ),
          segment(
            "s3",
            "sp-linh",
            29_000,
            36_000,
            "Ok, vậy tuần sau Minh gửi bản ngân sách chi tiết nhé.",
          ),
          segment(
            "s4",
            "sp-minh",
            36_500,
            44_000,
            "Được, mình sẽ gửi trước thứ Sáu. Còn vé máy bay đi da nang thì ai đặt?",
          ),
        ],
      },
    },
  );
  const sync = build(
    "m-notes",
    "Product sync tuần 39",
    new Date(2026, 8, 30, 15, 0).getTime(),
    31,
    { kind: "synced" },
    {
      people: [
        ["Linh", 1],
        ["Minh", 2],
        ["Me", 3],
      ],
      summary: "Beta ships 15 Oct; budget approved.",
      speakers: spk,
      transcript: {
        version: 2,
        // A decision mark on the first line (the notes cover it) and a question on the third (nothing does).
        marks: [
          { tMs: 91_000, tag: "decision", segment: "t1" },
          { tMs: 106_000, tag: "question", segment: "t3" },
        ],
        topics: [],
        segments: [
          segment(
            "t1",
            "sp-linh",
            90_000,
            96_000,
            "We can ship the beta on the fifteenth if QA signs off by Friday.",
          ),
          segment(
            "t2",
            "sp-minh",
            96_500,
            104_000,
            "Chốt scope cho bản beta: no sync, no Android, just the iPhone recorder.",
          ),
          segment(
            "t3",
            "sp-me",
            105_000,
            112_000,
            "I will send the revised budget in đồng and dollars before the review.",
          ),
          // The rest of the half hour, so the audio bar's waveform shows who spoke when.
          ...[
            [300_000, "sp-me"],
            [520_000, "sp-linh"],
            [760_000, "sp-minh"],
            [980_000, "sp-me"],
            [1_260_000, "sp-linh"],
            [1_540_000, "sp-minh"],
          ].map(([t0, who], i) =>
            segment(`t${i + 4}`, who as string, t0 as number, (t0 as number) + 150_000, "Noted, moving on to the next item."),
          ),
        ],
      },
      notes: {
        sections: [],
        marks: [
          { tMs: 91_000, tag: "decision", segment: "t1", text: "We can ship the beta on the fifteenth if QA signs off by Friday.", coveredBy: ["n1"] },
          { tMs: 106_000, tag: "question", segment: "t3", text: "I will send the revised budget in đồng and dollars before the review.", coveredBy: [] },
        ],
        blocks: [
          {
            gid: "n1",
            kind: "tldr",
            origin: "ai",
            text: "The team will ship the beta on 15 October if QA signs off by Friday.",
            pinned: false,
            citations: [
              cite(
                90_000,
                "We can ship the beta on the fifteenth if QA signs off by Friday.",
                { speakerGid: "sp-linh" },
              ),
            ],
          },
          {
            gid: "n2",
            kind: "decision",
            origin: "aiEdited",
            text: "Scope for the beta: iPhone recorder only; no sync, no Android.",
            pinned: false,
            citations: [
              cite(
                96_500,
                "Chốt scope cho bản beta: no sync, no Android, just the iPhone recorder.",
                { speakerGid: "sp-minh" },
              ),
            ],
          },
          {
            gid: "n3",
            kind: "question",
            origin: "ai",
            text: "Who confirms the QA sign-off date?",
            pinned: false,
            citations: [cite(200_000, "", { missing: true, t1Ms: null })],
          },
          {
            gid: "n-ask",
            kind: "answer",
            origin: "ai",
            text: "Q: When does the beta ship?\nA: On 15 October if QA signs off by Friday.",
            pinned: true,
            citations: [cite(105_000, "I will send the revised budget in đồng and dollars before the review.", { speakerGid: "sp-me" })],
          },
          {
            gid: "n4",
            kind: "note",
            origin: "user",
            text: "Ping legal about the voice profile wording.",
            pinned: false,
            citations: [],
          },
          {
            gid: "n5",
            kind: "enhanced:n4",
            origin: "ai",
            text: "",
            pinned: false,
            citations: [],
          },
        ] satisfies NoteBlockView[],
        actionItems: [
          {
            gid: "a1",
            text: "Send the revised budget before the review",
            ownerSpeakerGid: "sp-me",
            dueText: "Friday",
            done: false,
            origin: "ai",
            citations: [
              cite(
                105_000,
                "I will send the revised budget in đồng and dollars before the review.",
                { speakerGid: "sp-me" },
              ),
            ],
          },
          {
            gid: "a2",
            text: "Confirm QA sign-off date",
            ownerSpeakerGid: null,
            dueText: null,
            done: true,
            origin: "user",
            citations: [],
          },
        ] satisfies ActionItemView[],
      },
    },
  );
  const processing = build(
    "m-proc",
    "Họp khách hàng ACME",
    today + 11 * 60 * MIN,
    58,
    { kind: "processingOnPhone", percent: 42 },
    {
      people: [["Linh", 1]],
      status: "processing",
      job: { kind: "final_pass", progress: 0.42, waitingForModels: false },
      speakers: spk,
    },
  );
  const failed = build(
    "m-fail",
    "Phỏng vấn ứng viên",
    today - DAY + 10 * 60 * MIN,
    25,
    { kind: "failed" },
    { status: "failed" },
  );
  const old1 = build(
    "m-old1",
    "Retro tháng 9",
    new Date(2026, 8, 12, 16, 0).getTime(),
    47,
    { kind: "processedOnPhone" },
    { people: [["Minh", 2]] },
  );
  const old2 = build(
    "m-old2",
    "Weekly 1:1",
    new Date(2026, 8, 12, 9, 15).getTime(),
    18,
    { kind: "waitingForModels" },
    { status: "recorded" },
  );
  return [processing, failed, sync, planning, old1, old2];
}

function many(): MockMeeting[] {
  const base = new Date(2026, 8, 30, 18, 0).getTime();
  return Array.from({ length: 1200 }, (_, i) =>
    build(
      `g${i}`,
      `Cuộc họp số ${i + 1}`,
      base - Math.floor(i / 3) * DAY - (i % 3) * 3 * 60 * MIN,
      10 + (i % 50),
      { kind: "processedOnPhone" },
      {
        people: [["Linh", 1 + (i % 8)]],
        summary: i % 2 ? "Tóm tắt ngắn của cuộc họp." : null,
      },
    ),
  );
}

const scenario = () =>
  typeof location === "undefined"
    ? null
    : new URLSearchParams(location.search).get("meetings");
const meetings: MockMeeting[] =
  scenario() === "empty" ? [] : scenario() === "many" ? many() : defaults();

const find = (id: string) => meetings.find((m) => m.row.gid === id);
const label = (m: MockMeeting, gid: string | null) => {
  const s = m.detail.speakers.find((x) => x.gid === gid);
  return s ? (s.name ?? `Speaker ${s.number}`) : "?";
};

function asText(m: MockMeeting, markdown: boolean): string {
  const h = (t: string) => (markdown ? `## ${t}` : t.toUpperCase());
  const out = [markdown ? `# ${m.row.title}` : m.row.title, ""];
  if (m.notes.blocks.length) {
    out.push(
      h("Notes"),
      ...m.notes.blocks
        .filter((b) => b.text)
        .map((b) => (markdown ? `- ${b.text}` : b.text)),
      "",
    );
  }
  out.push(
    h("Transcript"),
    ...m.transcript.segments.map((s) => `${label(m, s.speakerGid)}: ${s.text}`),
  );
  return out.join("\n");
}

function highlights(text: string, query: string): [number, number][] {
  const f = fold(text);
  const q = fold(query).trim();
  const out: [number, number][] = [];
  if (!q) return out;
  for (let at = f.indexOf(q); at >= 0; at = f.indexOf(q, at + q.length))
    out.push([at, at + q.length]);
  return out;
}

/** What the native share sheet was asked to present (tests read it). */
export const shared: { meeting: string; format: string }[] = [];
if (typeof window !== "undefined")
  (window as unknown as { __ghiShared: typeof shared }).__ghiShared = shared;

let failDeletes = false;
let failSaves = false;
let failShare = false;
let failAudio = false;
/** Failure switches for the tests (the app lock itself is mock-settings'). */
if (typeof window !== "undefined")
  (window as unknown as { __ghiMeetings: unknown }).__ghiMeetings = {
    failDeletes: (v: boolean) => (failDeletes = v),
    failSaves: (v: boolean) => (failSaves = v),
    failShare: (v: boolean) => (failShare = v),
    failAudio: (v: boolean) => (failAudio = v),
    /** The meeting's processing ends (the core's state event follows separately). */
    finishProcessing: (id: string) => {
      const m = find(id);
      if (m) {
        m.detail.status = "ready";
        m.detail.job = null;
      }
    },
  };

export const meetingCommands: Partial<Commands> = {
  listMeetings: async (limit, offset) =>
    ok(meetings.slice(offset, offset + limit).map((m) => m.row)),
  meetingChips: async (ids) =>
    ok(
      ids.flatMap((gid) => (find(gid) ? [{ gid, chip: find(gid)!.chip }] : [])),
    ),
  meetingDetail: async (id) => {
    const m = find(id);
    return m ? ok(m.detail) : fail("not found");
  },
  meetingNotes: async (id) => {
    const m = find(id);
    return m ? ok(m.notes) : fail("not found");
  },
  meetingTranscript: async (id) => {
    const m = find(id);
    if (!m) return fail("not found");
    // Generated meetings get a short transcript on first open.
    if (!m.transcript.segments.length && id.startsWith("g"))
      m.transcript.segments = [
        segment(`${id}-1`, null, 1_000, 6_000, "Nội dung cuộc họp ở đây."),
      ];
    return ok(m.transcript);
  },
  // Like ghi-app's retranscribe: audio kept here, nothing running; the mock
  // records the language and the meeting is processing again.
  retranscribe: async (id, language) => {
    const m = find(id);
    if (!m) return fail("not found");
    if (m.detail.sensitive || !m.detail.audioAvailable) return fail("the meeting's audio is no longer kept");
    if (m.detail.job || m.detail.status === "processing") return fail("the meeting is still being processed");
    m.detail.language = language === "auto" ? null : language;
    m.detail.status = "processing";
    return ok(false);
  },
  updateSegmentText: async (id, seg, text) => {
    if (failSaves) return fail("disk full");
    const s = find(id)?.transcript.segments.find((x) => x.gid === seg);
    if (!s) return fail("not found");
    s.text = text;
    s.edited = true;
    s.words = [];
    return ok(null);
  },
  // Same rules and error codes as ghi-app's speakers_cmd; the lines follow their speakers.
  mergeMeetingSpeakers: async (id, from, into) => {
    const m = find(id);
    if (!m) return fail("not found");
    const f = m.detail.speakers.find((x) => x.gid === from);
    const t = m.detail.speakers.find((x) => x.gid === into);
    if (!f || !t) return fail("notASpeaker");
    if (f === t) return fail("sameSpeaker");
    // The phone records the mic only: no far side, so Me merges either way.
    for (const g of m.transcript.segments) if (g.speakerGid === from) g.speakerGid = into;
    for (const a of m.notes.actionItems) if (a.ownerSpeakerGid === from) a.ownerSpeakerGid = into;
    t.lines += f.lines;
    if (f.isMe) Object.assign(t, { isMe: true, notPerson: false });
    m.detail.speakers = m.detail.speakers.filter((x) => x !== f);
    return ok(null);
  },
  splitMeetingSpeaker: async (id, target, segmentGids, fromSegment) => {
    const m = find(id);
    if (!m) return fail("not found");
    const sp = m.detail.speakers.find((x) => x.gid === target);
    if (!sp) return fail("notASpeaker");
    const own = m.transcript.segments.filter((g) => g.speakerGid === target);
    const from = own.find((g) => g.gid === fromSegment);
    if ((segmentGids.length > 0) === (fromSegment != null) || (fromSegment != null && !from))
      return fail("nothingToSplit");
    const moving = from
      ? own.filter((g) => (g.t0Ms ?? 0) >= (from.t0Ms ?? 0))
      : segmentGids.map((g) => own.find((x) => x.gid === g));
    if (moving.some((g) => !g) || new Set(moving).size !== moving.length)
      return fail("nothingToSplit");
    if (moving.length >= own.length) return fail("wholeSpeaker");
    const spk = m.detail.speakers;
    const created = speaker(
      `sp-new-${spk.length + 1}`,
      Math.max(...spk.map((x) => x.number)) + 1,
      null,
      [1, 2, 3, 4, 5, 6, 7, 8].find((n) => !spk.some((x) => x.colorSlot === n)) ?? 0,
      false,
      moving.length,
    );
    created.notPerson = sp.notPerson;
    for (const g of moving) if (g) g.speakerGid = created.gid;
    sp.lines -= moving.length;
    spk.push(created);
    return ok(created.gid);
  },
  setSpeakerNotPerson: async (id, speaker, notPerson) => {
    const sp = find(id)?.detail.speakers.find((x) => x.gid === speaker);
    if (!sp) return fail("notASpeaker");
    if (notPerson && sp.isMe) return fail("isMe");
    sp.notPerson = notPerson;
    if (notPerson) sp.suggestion = null;
    return ok(null);
  },
  setActionDone: async (id, item, done) => {
    const a = find(id)?.notes.actionItems.find((x) => x.gid === item);
    if (!a) return fail("not found");
    a.done = done;
    return ok(null);
  },
  setMeetingCloudLocked: async (id, value) => {
    const m = find(id);
    if (!m) return fail("not found");
    m.detail.cloudLocked = value;
    return ok(null);
  },
  // On deletes the audio (the UI asked first); off only clears the flag.
  setMeetingSensitive: async (id, value) => {
    const m = find(id);
    if (!m) return fail("not found");
    m.detail.sensitive = value;
    m.row.sensitive = value;
    if (value) m.detail.audioAvailable = false;
    return ok(null);
  },
  deleteMeeting: async (id) => {
    if (failDeletes) return fail("disk full");
    const at = meetings.findIndex((m) => m.row.gid === id);
    if (at < 0) return fail("not found");
    meetings.splice(at, 1);
    return ok(null);
  },
  retryMeeting: async (id) => {
    const m = find(id);
    if (!m) return fail("not found");
    m.chip = { kind: "processingOnPhone", percent: 0 };
    m.row.status = "processing";
    return ok(1);
  },
  issueAudioPlay: async (id) => {
    if (failAudio) return fail("the app is starting");
    const m = find(id);
    return m
      ? ok({ token: `mock-audio-${id}`, durationMs: m.row.durationMs })
      : fail("not found");
  },
  // Loudness per 100 ms: loud where a line is said, quiet between (stable, no randomness).
  waveformPeaks: async (id) => {
    const m = find(id);
    if (!m) return fail("not found");
    const n = Math.max(1, Math.round(((m.row.durationMs ?? 60_000) / 1000) * 10));
    const peaks = Array.from({ length: n }, (_, i) => {
      const t = i * 100;
      const speaking = m.transcript.segments.some((s) => (s.t0Ms ?? 0) <= t && (s.t1Ms ?? 0) > t);
      // Bursts of speech between pauses, so a long meeting looks like one.
      const burst = i % 70 < 48;
      return speaking || burst ? 120 + Math.round(110 * Math.abs(Math.sin(i / 5) * Math.cos(i / 17))) : 24;
    });
    return ok({ perSecond: 10, peaks });
  },
  shareMeetingExport: async (id, format) => {
    if (failShare) return fail("share sheet unavailable");
    const m = find(id);
    if (!m) return fail("not found");
    shared.push({ meeting: id, format });
    return ok(null);
  },
  meetingAsText: async (id, markdown) => {
    const m = find(id);
    return m ? ok(asText(m, markdown)) : fail("not found");
  },
  searchMeetings: async (request) => {
    const q = request.text.trim();
    if (!q) return ok({ hits: [], truncated: false });
    const hits: SearchHitView[] = [];
    for (const m of meetings) {
      for (const s of m.transcript.segments) {
        const h = highlights(s.text, q);
        if (h.length)
          hits.push({
            kind: "segment",
            meeting: m.row.gid,
            meetingTitle: m.row.title,
            meetingStartedAt: m.row.startedAt,
            item: s.gid,
            speakerGid: s.speakerGid,
            t0Ms: s.t0Ms,
            t1Ms: s.t1Ms,
            snippet: s.text,
            highlights: h,
            exact: s.text.toLowerCase().includes(q.toLowerCase()),
          });
      }
      for (const b of m.notes.blocks) {
        const h = highlights(b.text, q);
        if (h.length)
          hits.push({
            kind: "note",
            meeting: m.row.gid,
            meetingTitle: m.row.title,
            meetingStartedAt: m.row.startedAt,
            item: b.gid,
            speakerGid: null,
            t0Ms: b.citations[0]?.t0Ms ?? null,
            t1Ms: null,
            snippet: b.text,
            highlights: h,
            exact: b.text.toLowerCase().includes(q.toLowerCase()),
          });
      }
    }
    hits.sort((a, b) => Number(b.exact) - Number(a.exact));
    return ok({
      hits: hits.slice(request.offset, request.offset + request.limit),
      truncated: false,
    });
  },
};
