// SPDX-License-Identifier: Apache-2.0

// The sample meeting behind the landing page's live demo and notes, one
// version per language, from the approved prototype. Plain data and pure
// helpers: no imports, so `node --test` can load it (demo-data.test.ts).
//
// The `COPIED` labels are copies of strings from packages/i18n/locales
// (the app's own words inside the demo window); the test fails when a copy
// drifts from en.json / vi.json. The timings and speakers must equal
// packages/ui/mocks/sample-meeting.json (same test).

export type Lang = "en" | "vi";
export const LANGS: readonly Lang[] = ["en", "vi"];
export const DEFAULT_LANG: Lang = "en";
/** localStorage key for the demo's language pick. */
export const LANG_KEY = "ghira-site-lang";

export type Text = Record<Lang, string>;

/** Seconds into the meeting at which the demo is drawn on the server. */
export const DEMO_START = 27;
/** Length of the sample meeting, in seconds. */
export const END = 101;
/** The demo plays this many meeting seconds per real second. */
export const SPEED = 2.4;
/** Tick of the demo clock, in seconds. */
export const STEP = 0.1;
/** The demo holds the finished meeting for a few seconds, then restarts. */
export const LOOP_AT = END + 6;
export const LOOP_RESTART = 2;
/** Typing speed of "Your notes", in characters per meeting second. */
const TYPING_RATE = 9;

/** Speaker colors are the design tokens --s1 … --s8 (slots), never alone: always with an initial and a name. */
export const SPEAKERS: readonly { name: string | null; slot: number }[] = [
  { name: null, slot: 1 }, // Me / Tôi
  { name: "Linh", slot: 2 },
  { name: "Minh", slot: 4 },
  { name: "Sarah", slot: 8 },
];

export interface Line {
  /** Start, seconds into the meeting. */
  t: number;
  /** Index into SPEAKERS. */
  s: number;
  text: Text;
  /** The words the speech model is unsure of (drawn with a dotted underline). */
  low?: Text;
}

export const LINES: readonly Line[] = [
  { t: 4, s: 0, text: { en: "Okay, let's start. Today we lock the scope for the November beta.", vi: "Okay, bắt đầu nhé. Hôm nay mình chốt phạm vi cho bản beta tháng 11." } },
  {
    t: 11,
    s: 1,
    text: {
      en: "I collected feedback from 12 user tests. The biggest problem is telling speakers apart in in-person meetings.",
      vi: "Em đã gom phản hồi từ 12 buổi thử với người dùng. Vấn đề lớn nhất là nhận diện người nói khi họp trực tiếp.",
    },
  },
  {
    t: 22,
    s: 2,
    text: { en: "The new Nemotron cuts errors a lot, but we can't export it to CoreML yet.", vi: "Bản Nemotron mới giảm lỗi nhiều, nhưng mình chưa xuất được sang CoreML." },
    low: { en: "CoreML", vi: "CoreML." },
  },
  {
    t: 31,
    s: 3,
    text: {
      en: "From the design side, people want to rename speakers during the call, not after.",
      vi: "Về phía thiết kế, mọi người muốn đổi tên người nói ngay trong cuộc họp, không phải sau đó.",
    },
  },
  { t: 38, s: 0, text: { en: "Agreed. Live rename has to be in the beta.", vi: "Đồng ý. Đổi tên trực tiếp phải có trong bản beta." } },
  {
    t: 44,
    s: 2,
    text: {
      en: "Renaming is easy. The hard part is remembering voices across meetings, so we need voice profiles.",
      vi: "Đổi tên thì dễ. Khó là nhớ giọng qua các cuộc họp, cần có hồ sơ giọng nói.",
    },
  },
  { t: 53, s: 1, text: { en: "Then we need a consent screen before saving anyone's voice.", vi: "Vậy mình cần màn hình xin đồng ý trước khi lưu giọng nói." } },
  {
    t: 61,
    s: 3,
    text: {
      en: "Can we also support importing Plaud recordings? Half of our testers own one.",
      vi: "Mình có hỗ trợ nhập bản ghi từ Plaud được không? Một nửa số người thử đang dùng máy này.",
    },
  },
  {
    t: 69,
    s: 0,
    text: { en: "Yes, file import is P0. Minh, can you estimate it for me?", vi: "Có, nhập file là ưu tiên số một. Minh ước lượng giúp anh được không?" },
  },
  { t: 75, s: 2, text: { en: "About two weeks, including a background processing queue.", vi: "Khoảng hai tuần, gồm cả hàng đợi xử lý nền." } },
  { t: 81, s: 1, text: { en: "The internal deadline is 15/11, I'll update the roadmap.", vi: "Hạn nội bộ là 15/11, em sẽ cập nhật lộ trình." } },
  { t: 88, s: 3, text: { en: "I'll send the updated speaker panel mockups by Friday.", vi: "Em sẽ gửi bản thiết kế mới của bảng người nói trước thứ Sáu." } },
  {
    t: 95,
    s: 0,
    text: {
      en: "Good. To wrap up: live rename, voice profiles with consent, file import.",
      vi: "Tốt. Chốt lại: đổi tên trực tiếp, hồ sơ giọng nói có xin đồng ý, nhập file.",
    },
  },
];

/** The notes the user typed during the call. */
export const MY_NOTES: readonly { t: number; text: Text }[] = [
  { t: 14, text: { en: "ask about pricing tiers", vi: "hỏi về các gói giá" } },
  { t: 34, text: { en: "rename speakers live?", vi: "đổi tên người nói trực tiếp?" } },
  { t: 63, text: { en: "plaud import", vi: "nhập từ plaud" } },
  { t: 96, text: { en: "beta scope nov", vi: "phạm vi beta t11" } },
];

/** A note sentence with the transcript lines it cites (indexes into LINES). */
export interface Cited {
  text: Text;
  c: readonly number[];
}
export interface Action extends Cited {
  /** Owner: index into SPEAKERS, or -1 for nobody yet. */
  o: number;
  due: Text;
}
/** One of the user's typed notes (index into MY_NOTES), filled in, or flagged as not discussed. */
export type Mine = { k: number; miss: true } | { k: number; miss?: false; text: Text; c: readonly number[] };

export const NOTES: {
  summary: readonly Cited[];
  decisions: readonly Cited[];
  actions: readonly Action[];
  mine: readonly Mine[];
  questions: readonly Cited[];
} = {
  summary: [
    {
      text: {
        en: "Beta scope for November is locked: live speaker rename, voice profiles with consent, and audio file import.",
        vi: "Chốt phạm vi bản beta tháng 11: đổi tên người nói trực tiếp, hồ sơ giọng nói có xin đồng ý, nhập file âm thanh.",
      },
      c: [12],
    },
    {
      text: { en: "Testers' biggest complaint is speaker detection in in-person meetings.", vi: "Phàn nàn lớn nhất từ người thử là nhận diện người nói khi họp trực tiếp." },
      c: [1],
    },
    {
      text: {
        en: "The new Nemotron model cuts diarization errors, but the CoreML export is not done yet.",
        vi: "Nemotron mới giảm lỗi phân tách người nói, nhưng chưa xuất được sang CoreML.",
      },
      c: [2],
    },
  ],
  decisions: [
    { text: { en: "Live speaker rename ships in the beta.", vi: "Đổi tên người nói trực tiếp có trong bản beta." }, c: [4] },
    { text: { en: "Voice profiles are saved only after a consent screen.", vi: "Chỉ lưu hồ sơ giọng nói sau màn hình xin đồng ý." }, c: [6] },
    { text: { en: "Audio file import, including Plaud, is P0.", vi: "Nhập file âm thanh, gồm cả Plaud, là ưu tiên số một." }, c: [8] },
  ],
  actions: [
    {
      o: 2,
      text: { en: "Build file import with a background processing queue", vi: "Làm tính năng nhập file kèm hàng đợi xử lý nền" },
      due: { en: "About 2 weeks", vi: "Khoảng 2 tuần" },
      c: [9],
    },
    {
      o: 1,
      text: { en: "Update the roadmap for the 15/11 internal deadline", vi: "Cập nhật lộ trình theo hạn nội bộ 15/11" },
      due: { en: "Due 15/11", vi: "Hạn 15/11" },
      c: [10],
    },
    {
      o: 3,
      text: { en: "Send updated speaker panel mockups", vi: "Gửi bản thiết kế mới của bảng người nói" },
      due: { en: "Due Friday", vi: "Hạn thứ Sáu" },
      c: [11],
    },
    {
      o: -1,
      text: { en: "Spec the voice consent screen", vi: "Viết đặc tả màn hình xin đồng ý giọng nói" },
      due: { en: "No owner yet", vi: "Chưa có người nhận" },
      c: [6],
    },
  ],
  mine: [
    {
      k: 1,
      text: {
        en: "Sarah: testers want to rename speakers during the call, not after. Agreed as a beta must-have.",
        vi: "Sarah: người thử muốn đổi tên người nói ngay trong cuộc họp. Đồng ý là bắt buộc cho bản beta.",
      },
      c: [3, 4],
    },
    {
      k: 2,
      text: {
        en: "Half of the testers own a Plaud, so import is P0. Minh estimates about two weeks, including a background queue.",
        vi: "Một nửa người thử dùng Plaud nên nhập file là ưu tiên số một. Minh ước lượng khoảng hai tuần, gồm cả hàng đợi xử lý nền.",
      },
      c: [7, 9],
    },
    {
      k: 3,
      text: {
        en: "Scope confirmed at the end of the call: live rename, consented voice profiles, file import.",
        vi: "Phạm vi được chốt cuối buổi: đổi tên trực tiếp, hồ sơ giọng nói có đồng ý, nhập file.",
      },
      c: [12],
    },
    { k: 0, miss: true },
  ],
  questions: [
    { text: { en: "When will the CoreML export of Nemotron be ready?", vi: "Khi nào xuất Nemotron sang CoreML xong?" }, c: [2] },
    { text: { en: "Which Plaud export formats does import need to support?", vi: "Cần hỗ trợ những định dạng xuất nào của Plaud?" }, c: [7] },
  ],
};

/**
 * The app's own words inside the demo window, copied from
 * packages/i18n/locales/{en,vi}.json. Keep in step: demo-data.test.ts
 * compares every value with its key in COPIED_KEYS.
 */
export interface CopiedLabels {
  local: string;
  recording: string;
  yourNotes: string;
  me: string;
  identifying: string;
  unassigned: string;
  notDiscussed: string;
  sections: { summary: string; yourNotes: string; decisions: string; actionItems: string; openQuestions: string };
}

export const COPIED: Record<Lang, CopiedLabels> = {
  en: {
    local: "Local only",
    recording: "Recording",
    yourNotes: "Your notes",
    me: "Me",
    identifying: "Identifying speaker…",
    unassigned: "Unassigned",
    notDiscussed: "Not discussed in this meeting",
    sections: { summary: "Summary", yourNotes: "Your notes, filled in", decisions: "Decisions", actionItems: "Action items", openQuestions: "Open questions" },
  },
  vi: {
    local: "Chỉ trên máy này",
    recording: "Đang ghi",
    yourNotes: "Ghi chú của bạn",
    me: "Tôi",
    identifying: "Đang nhận diện người nói…",
    unassigned: "Chưa giao",
    notDiscussed: "Không được nhắc tới trong cuộc họp này",
    sections: { summary: "Tóm tắt", yourNotes: "Ghi chú của bạn, đã bổ sung", decisions: "Quyết định", actionItems: "Việc cần làm", openQuestions: "Câu hỏi mở" },
  },
};

/** The locale key each COPIED label is copied from (checked by the test). */
export const COPIED_KEYS = {
  local: "privacy.local",
  recording: "library.status.recording",
  yourNotes: "live.yourNotes",
  me: "speakers.me",
  identifying: "speakers.identifying",
  unassigned: "notes.unassigned",
  notDiscussed: "ask.meeting.notDiscussed",
  "sections.summary": "notes.sections.summary",
  "sections.yourNotes": "notes.sections.yourNotes",
  "sections.decisions": "notes.sections.decisions",
  "sections.actionItems": "notes.sections.actionItems",
  "sections.openQuestions": "notes.sections.openQuestions",
} as const;

/** Words of the demo that are the sample's own (not in the app's locale files). */
export const SAMPLE_LABELS: Record<Lang, { title: string; hint: string; transcript: string; meta: string; showLine: string }> = {
  en: {
    title: "Beta scope check-in, Zoom",
    hint: "Type a few words while you listen. Ghira turns them into full notes after the call.",
    transcript: "Transcript",
    meta: "4 speakers, 1:41",
    showLine: "Show the line at",
  },
  vi: {
    title: "Chốt phạm vi bản beta, Zoom",
    hint: "Gõ vài chữ trong lúc nghe. Ghira viết thành ghi chú đầy đủ sau cuộc họp.",
    transcript: "Bản ghi lời",
    meta: "4 người nói, 1:41",
    showLine: "Xem câu nói lúc",
  },
};

/** The speaker's name; the user ("Me" / "Tôi") has none of their own. */
export function speakerName(s: number, lang: Lang): string {
  return SPEAKERS[s].name ?? COPIED[lang].me;
}

/** `m:ss` as `00:28`. */
export function clock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

export function wordsOf(i: number, lang: Lang): string[] {
  return LINES[i].text[lang].split(/\s+/);
}

/** How long line `i` lasts: until the next line starts. */
export function lineDuration(i: number): number {
  return (LINES[i + 1] ? LINES[i + 1].t : END) - LINES[i].t;
}

export interface Shown {
  i: number;
  /** Words on screen. */
  shown: number;
  /** Still being recognized (drawn grey, last words fading in). */
  partial: boolean;
}

/** Every line that has started at `now`, with how much of it is on screen. */
export function shownLines(now: number, lang: Lang): Shown[] {
  const out: Shown[] = [];
  LINES.forEach((line, i) => {
    if (now < line.t) return;
    const words = wordsOf(i, lang).length;
    const p = Math.min(1, (now - line.t) / (lineDuration(i) * 0.82));
    out.push({ i, shown: Math.max(1, Math.ceil(words * p)), partial: p < 1 });
  });
  return out;
}

/** A new voice shows "Identifying speaker…" for its first words, as in the app. */
export function isIdentifying(s: Shown): boolean {
  return s.partial && s.shown <= 3 && s.i === 3;
}

/** The speech-activity bars of speaker `s`: left and width, in percent of the time axis. */
export function laneSegments(s: number, now: number): { left: number; width: number }[] {
  const axis = Math.max(now, 24);
  const out: { left: number; width: number }[] = [];
  LINES.forEach((line, i) => {
    if (line.s !== s || now < line.t) return;
    const end = Math.min(now, line.t + lineDuration(i) - 0.8);
    out.push({ left: (line.t / axis) * 100, width: Math.max(0.6, ((end - line.t) / axis) * 100) });
  });
  return out;
}

/** The typed notes at `now`, each cut to what has been typed so far. */
export function typedNotes(now: number, lang: Lang): { text: string; typing: boolean }[] {
  return MY_NOTES.filter((m) => now >= m.t).map((m) => {
    const text = m.text[lang];
    const n = Math.min(text.length, Math.floor((now - m.t) * TYPING_RATE));
    return { text: text.slice(0, n), typing: n < text.length };
  });
}

/** Meeting time after one tick of the demo clock. */
export function nextTime(now: number): number {
  const next = now + STEP * SPEED;
  return next > LOOP_AT ? LOOP_RESTART : next;
}
