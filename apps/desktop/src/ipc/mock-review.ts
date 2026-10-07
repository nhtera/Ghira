// SPDX-License-Identifier: Apache-2.0
// The mock core's review side (phase 11): meeting detail, notes and
// transcript built from the sample meeting (brief §10), their edits in
// memory, search with VN folding, templates, regenerate, a silent playable
// WAV for the audio bar, export (pretend-saved) and the import queue.
import sample from "@ghi/ui/mocks/sample-meeting.json";
import importFiles from "@ghi/ui/mocks/import-files.json";
import type {
  ActionItemView,
  Citation,
  ImportStaged,
  ImportUpdate,
  MeetingDetail,
  MeetingNotes,
  MeetingRow,
  MeetingSpeaker,
  MeetingTranscript,
  NoteBlockView,
  SearchHitView,
  SegmentView,
  StagedFile,
  TemplateInfo,
} from "../bindings";
import type { Commands } from "./ipc";
import { audioDeleted, lockedMeetings, sensitiveMeetings } from "./mock-ai";

type Result<T> = { status: "ok"; data: T } | { status: "error"; error: string };
const ok = <T>(data: T): Promise<Result<T>> => Promise.resolve({ status: "ok", data });
const fail = <T>(error: string): Promise<Result<T>> => Promise.resolve({ status: "error", error });

/** What the review commands need from the rest of the mock. */
export interface ReviewHost {
  rows: MeetingRow[];
  /** Pretends the notes job ran (processing → ready, notesReady). */
  process(meeting: string): void;
}

const SPEAKER_NAMES = ["An Tran", "Sarah", "Minh", "Linh", "Jordan", "Priya"];
const SLOTS = [1, 2, 4, 8, 3, 5];
/** `?suggest=1`: one unnamed speaker per meeting "sounds like Me" (the final pass's voice match). */
const SUGGEST_SPEAKER = 2;
const withSuggestion = () => new URLSearchParams(location.search).has("suggest");
/** `?overlap=1`: lines 4 and 5 (two speakers, clear of any topic header) talk over each other for a few seconds. */
const withOverlap = () => new URLSearchParams(location.search).has("overlap");
const OVERLAP_LINES = [4, 5];
const OVERLAP_MS = 4000;

type Detail = { speakers: MeetingSpeaker[]; notes: MeetingNotes; transcript: MeetingTranscript };
const details = new Map<string, Detail>();
let seq = 0;
const gid = (p: string) => `${p}-${++seq}`;

const starts = sample.lineStartSeconds.map((s) => s * 1000);
const endOf = (i: number) => starts[i + 1] ?? sample.durationSeconds * 1000;

function segments(): SegmentView[] {
  return sample.transcript.map((l, i) => {
    const overlap = withOverlap() && OVERLAP_LINES.includes(i);
    // The second of the two starts before the first has finished.
    const t0 = i === OVERLAP_LINES[1] && overlap ? endOf(i - 1) - 400 - OVERLAP_MS : (starts[i] ?? 0);
    const t1 = endOf(i) - 400;
    const words = l.x.split(" ");
    const step = (t1 - t0) / words.length;
    return {
      gid: `seg-${i}`,
      speakerGid: `spk-${l.s}`,
      t0Ms: t0,
      t1Ms: t1,
      text: l.x,
      language: "vi",
      confidence: 0.92,
      edited: false,
      overlap,
      words: words.map((w, k) => ({
        t0Ms: t0 + k * step,
        t1Ms: t0 + (k + 1) * step,
        confidence: "low" in l && l.low === w ? 0.35 : 0.95,
      })),
    };
  });
}

function cite(segs: SegmentView[], ids: number[]): Citation[] {
  return ids.map((i) => {
    const s = segs[i];
    return {
      t0Ms: s?.t0Ms ?? 0,
      t1Ms: s?.t1Ms ?? 0,
      quote: s?.text ?? "",
      speakerGid: s?.speakerGid ?? null,
      stale: false,
      missing: !s,
    };
  });
}

type Item = { en?: string; vi?: string; c?: number[]; edited?: boolean; me?: string; miss?: boolean; owner?: number; due?: string | [string, string]; done?: boolean };

function build(lang: "en" | "vi"): Detail {
  const segs = segments();
  const used = [...new Set(sample.transcript.map((l) => l.s))];
  const speakers: MeetingSpeaker[] = used.map((s, i) => ({
    gid: `spk-${s}`,
    name: s === SUGGEST_SPEAKER && withSuggestion() ? null : (SPEAKER_NAMES[s] ?? null),
    number: i + 1,
    colorSlot: SLOTS[s % SLOTS.length]!,
    isMe: s === 0,
    notPerson: false,
    lines: sample.transcript.filter((l) => l.s === s).length,
    sampleT0Ms: segs.find((g) => g.speakerGid === `spk-${s}`)?.t0Ms ?? null,
    sampleT1Ms: (segs.find((g) => g.speakerGid === `spk-${s}`)?.t0Ms ?? 0) + 3000,
    suggestion: s === SUGGEST_SPEAKER && withSuggestion() ? { personGid: "person-me", name: "", isMe: true, score: 0.74 } : null,
  }));
  const n = sample.notes as unknown as Record<string, Item[]>;
  const text = (x: Item) => (lang === "vi" ? x.vi : x.en) ?? x.en ?? "";
  const blocks: NoteBlockView[] = [];
  const ai = (kind: string, items: Item[] | undefined) =>
    (items ?? []).forEach((x) =>
      blocks.push({ gid: gid("blk"), kind, origin: x.edited ? "aiEdited" : "ai", text: text(x), pinned: false, citations: cite(segs, x.c ?? []) }),
    );
  ai("tldr", n.summary);
  ai("decision", n.decisions);
  ai("question", n.openQuestions);
  ai("quote", n.keyQuotes);
  for (const [t, en, vi] of sample.topics as [number, string, string][]) {
    if (t * 1000 >= sample.durationSeconds * 1000) continue;
    const i = Math.max(0, starts.findIndex((s, k) => s <= t * 1000 && endOf(k) > t * 1000));
    blocks.push({ gid: gid("blk"), kind: "topic", origin: "ai", text: lang === "vi" ? vi : en, pinned: false, citations: cite(segs, [i]) });
  }
  for (const x of n.yourNotes ?? []) {
    const user = gid("blk");
    blocks.push({ gid: user, kind: "note", origin: "user", text: x.me ?? "", pinned: false, citations: [] });
    blocks.push({
      gid: gid("blk"),
      kind: `enhanced:${user}`,
      origin: "ai",
      text: x.miss ? "" : text(x),
      pinned: false,
      citations: x.miss ? [] : cite(segs, x.c ?? []),
    });
  }
  const actionItems: ActionItemView[] = (n.actionItems ?? []).map((x) => ({
    gid: gid("act"),
    text: text(x),
    ownerSpeakerGid: x.owner != null ? `spk-${x.owner}` : null,
    // The sample stores the due words as [English, Vietnamese]: only the interface language is shown, none if empty.
    dueText: (Array.isArray(x.due) ? x.due[lang === "vi" ? 1 : 0] : x.due) || null,
    done: !!x.done,
    origin: x.edited ? "aiEdited" : "ai",
    citations: cite(segs, x.c ?? []),
  }));
  const topics = blocks
    .filter((b) => b.kind === "topic")
    .map((b) => ({ title: b.text, tMs: b.citations[0]?.t0Ms ?? 0 }))
    .sort((a, b) => (a.tMs ?? 0) - (b.tMs ?? 0));
  return {
    speakers,
    notes: { blocks, actionItems, sections: [] },
    transcript: {
      version: 2,
      segments: segs,
      marks: [{ tMs: starts[4] ?? 0, tag: "decision" }],
      topics,
    },
  };
}

function detailOf(meeting: string): Detail {
  let d = details.get(meeting);
  if (!d) {
    d = build(document.documentElement.lang === "vi" ? "vi" : "en");
    details.set(meeting, d);
  }
  return d;
}

/** VN-folded, one char per char (so indexes map back to the original). */
const fold = (s: string) =>
  [...s]
    .map((c) =>
      c
        .normalize("NFD")
        .replace(/[̀-ͯ]/g, "")
        .replace(/[đĐ]/, "d")
        .toLowerCase()
        .charAt(0) || c,
    )
    .join("");

function highlights(text: string, query: string): [number, number][] {
  const t = fold(text);
  const out: [number, number][] = [];
  for (const w of fold(query).split(/\s+/).filter(Boolean)) {
    let i = t.indexOf(w);
    while (i >= 0) {
      out.push([i, i + w.length]);
      i = t.indexOf(w, i + w.length);
    }
  }
  return out.sort((a, b) => a[0] - b[0]);
}

/** A silent 8 kHz WAV as long as the meeting (the browser can seek it). */
function silentWav(ms: number): string {
  const n = Math.round((ms / 1000) * 8000);
  const b = new Uint8Array(44 + n);
  const v = new DataView(b.buffer);
  const ascii = (o: number, s: string) => [...s].forEach((c, i) => (b[o + i] = c.charCodeAt(0)));
  ascii(0, "RIFF");
  v.setUint32(4, 36 + n, true);
  ascii(8, "WAVEfmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, 1, true);
  v.setUint32(24, 8000, true);
  v.setUint32(28, 8000, true);
  v.setUint16(32, 1, true);
  v.setUint16(34, 8, true);
  ascii(36, "data");
  v.setUint32(40, n, true);
  b.fill(128, 44);
  return URL.createObjectURL(new Blob([b], { type: "audio/wav" }));
}
const audio = new Map<string, string>();

const TEMPLATES: TemplateInfo[] = [
  { id: "general", name: "General meeting", sections: [] },
  { id: "one_on_one", name: "1:1", sections: [] },
  { id: "standup", name: "Standup", sections: [] },
  {
    id: "client",
    name: "Client meeting",
    sections: [
      { id: "requests", titleEn: "Client requests", titleVi: "Yêu cầu của khách hàng" },
      { id: "feedback", titleEn: "Client feedback", titleVi: "Phản hồi của khách hàng" },
    ],
  },
  { id: "sales", name: "Sales call", sections: [] },
  { id: "interview", name: "Interview", sections: [] },
  { id: "lecture", name: "Lecture", sections: [] },
];

// Import: staged files from the design's sample list.
const staged = new Map<string, StagedFile>();
const stagedListeners = new Set<(e: ImportStaged) => void>();
const updateListeners = new Set<(e: ImportUpdate) => void>();
const cancelled = new Set<string>();
const emitUpdate = (u: ImportUpdate) => updateListeners.forEach((l) => l(u));

const parseMin = (dur: string) => {
  const h = /(\d+)\s*h/.exec(dur)?.[1];
  const m = /(\d+)\s*min/.exec(dur)?.[1];
  return (Number(h ?? 0) * 60 + Number(m ?? 0)) * 60_000;
};
const parseSize = (size: string) => {
  const [n, u] = size.split(" ");
  return Number(n) * (u === "GB" ? 1e9 : 1e6);
};

function stageSamples(): StagedFile[] {
  return importFiles.files.map((f) => {
    const s: StagedFile = {
      id: gid("stage"),
      name: f.name,
      sizeBytes: parseSize(f.size),
      durationMs: f.kind === "corrupt" ? null : parseMin(f.dur),
      channels: "stereo" in f && f.stereo ? 2 : 1,
      source: f.src.startsWith("Plaud") ? "plaud" : f.src.startsWith("Zoom") ? "zoom" : f.src === "Voice Memos" ? "voiceMemos" : "other",
      problems:
        f.kind === "corrupt" ? ["unsupported"] : f.kind === "long" ? ["veryLong"] : f.kind === "dup" ? ["duplicate"] : [],
      duplicateOf: f.kind === "dup" ? { meeting: "sample-1", title: "Client call — Acme onboarding" } : null,
      group: null,
      participant: null,
      title: null,
      startedAt: null,
    };
    staged.set(s.id, s);
    return s;
  });
}

/**
 * `?importgroup=1` (or `=dup`): the open dialog returns a Zoom meeting's
 * per-participant tracks (one unnamed), the mixed recording that is then not
 * imported, and a stray file; `=dup` marks the group as imported before.
 */
const importGroup = () => new URLSearchParams(location.search).get("importgroup");

/** The mixed recording is left out while tracks of its recording are staged (and back when none is). */
function refreshSuperseded(): StagedFile[] {
  const hasTracks = [...staged.values()].some((f) => f.group);
  const changed: StagedFile[] = [];
  for (const f of staged.values()) {
    if (f.group || f.source !== "zoom" || !f.name.startsWith("audio_only")) continue;
    const marked = f.problems.includes("superseded");
    if (hasTracks && !marked) f.problems = [...f.problems, "superseded"];
    else if (!hasTracks && marked) f.problems = f.problems.filter((p) => p !== "superseded");
    else continue;
    changed.push({ ...f });
  }
  return changed;
}

function stageZoomGroup(): StagedFile[] {
  const group = gid("group");
  const dup = importGroup() === "dup";
  const track = (name: string, participant: string | null): StagedFile => ({
    id: gid("stage"),
    name,
    sizeBytes: 24_000_000,
    durationMs: 3_540_000,
    channels: 1,
    source: "zoom",
    problems: dup ? ["duplicate"] : [],
    duplicateOf: dup ? { meeting: "sample-1", title: "Sprint planning" } : null,
    group,
    participant,
    title: "Sprint planning",
    startedAt: Date.now() - 86_400_000,
  });
  const files: StagedFile[] = [
    track("audioLinh1111.m4a", "Linh"),
    track("audioMinh2222.m4a", "Minh"),
    track("audioSarah3333.m4a", "Sarah"),
    track("audio_recording_4.m4a", null),
    {
      id: gid("stage"),
      name: "audio_only.m4a",
      sizeBytes: 48_000_000,
      durationMs: 3_540_000,
      channels: 1,
      source: "zoom",
      problems: ["superseded"],
      duplicateOf: null,
      group: null,
      participant: null,
      title: "Sprint planning",
      startedAt: Date.now() - 86_400_000,
    },
  ];
  files.forEach((f) => staged.set(f.id, f));
  return files;
}

/** Dropped files not yet read by the import screen. */
let dropped: string[] = [];

/** Dev/test hook: files dropped on the window. */
export function simulateImportDrop() {
  const files = stageSamples().slice(0, 3);
  dropped.push(...files.map((f) => f.id));
  stagedListeners.forEach((l) => l({ files: files.map((f) => ({ id: f.id })) }));
}

export const onImportStaged = (cb: (e: ImportStaged) => void) => {
  stagedListeners.add(cb);
  return Promise.resolve(() => void stagedListeners.delete(cb));
};
export const onImportUpdate = (cb: (e: ImportUpdate) => void) => {
  updateListeners.add(cb);
  return Promise.resolve(() => void updateListeners.delete(cb));
};
export const audioUrl = (token: string) => audio.get(token) ?? "";
/** For the AI mock: a meeting's transcript and speaker names. */
/** The meeting's stored speakers (mutable: the mock's voice commands edit them). */
export const speakersOf = (m: string) => detailOf(m).speakers;
export const transcriptOf = (m: string) => detailOf(m).transcript;
export const namesOf = (m: string) => detailOf(m).speakers.flatMap((s) => (s.name ? [s.name] : []));

type ReviewCommands = Pick<
  Commands,
  | "meetingDetail"
  | "meetingNotes"
  | "meetingTranscript"
  | "updateSegmentText"
  | "setSegmentSpeaker"
  | "renameMeetingSpeaker"
  | "mergeMeetingSpeakers"
  | "splitMeetingSpeaker"
  | "setSpeakerNotPerson"
  | "updateNoteBlock"
  | "addNoteBlock"
  | "deleteNoteBlock"
  | "addActionItem"
  | "updateActionItem"
  | "setActionDone"
  | "setActionOwner"
  | "deleteActionItem"
  | "listTemplates"
  | "regenerateNotes"
  | "retranscribe"
  | "searchMeetings"
  | "issueAudioPlay"
  | "waveformPeaks"
  | "exportMeeting"
  | "exportMeetings"
  | "exportObsidian"
  | "meetingAsText"
  | "revealLastExport"
  | "exportDestination"
  | "chooseExportFolder"
  | "obsidianVault"
  | "chooseObsidianVault"
  | "pickImportFiles"
  | "stagedFiles"
  | "takeDroppedFiles"
  | "unstageFiles"
  | "importTracksSeparately"
  | "startImport"
  | "cancelImport"
>;

/** The remembered export folder (its name only). */
let exportFolder: string | null = "Documents";
/** The spoken language a meeting was transcribed again in (`null`: English + Vietnamese). */
const languages = new Map<string, string | null>();
let obsidianVault: string | null = "Vault";

export function reviewCommands(host: ReviewHost): ReviewCommands {
  const row = (m: string) => host.rows.find((r) => r.gid === m);
  const withDetail = <T>(m: string, f: (d: Detail) => T): Promise<Result<T>> =>
    row(m) ? ok(f(detailOf(m))) : fail(`meeting not found: ${m}`);
  const block = (d: Detail, b: string) => d.notes.blocks.find((x) => x.gid === b);
  const item = (d: Detail, a: string) => d.notes.actionItems.find((x) => x.gid === a);
  const seg = (d: Detail, s: string) => d.transcript.segments.find((x) => x.gid === s);

  return {
    meetingDetail: (m) => {
      const r = row(m);
      if (!r) return fail(`meeting not found: ${m}`);
      const d = detailOf(m);
      const detail: MeetingDetail = {
        gid: r.gid,
        title: r.title,
        startedAt: r.startedAt,
        durationMs: sample.durationSeconds * 1000,
        source: r.source,
        mode: r.mode,
        language: languages.has(r.gid) ? (languages.get(r.gid) ?? null) : "en",
        template: r.template,
        status: r.status,
        cloudLocked: lockedMeetings.has(r.gid),
        sensitive: sensitiveMeetings.has(r.gid),
        cloudUsed: r.cloudUsed,
        consentConfirmed: r.consentConfirmed,
        transcriptVersion: r.transcriptVersion,
        audioAvailable: !audioDeleted.has(r.gid),
        // Copies: the voice commands edit the stored speakers in place, and an unchanged reference would hide that from the query cache.
        speakers: d.speakers.map((s) => ({ ...s })),
        job: r.job,
        notesModel: r.cloudUsed ? null : "Qwen3-8B",
        sourceApp: r.source === "live" && r.mode === "call" ? "zoom" : null,
      };
      return ok(detail);
    },
    meetingNotes: (m) => withDetail(m, (d) => structuredClone(d.notes)),
    meetingTranscript: (m) => withDetail(m, (d) => structuredClone(d.transcript)),
    updateSegmentText: (m, s, text) =>
      withDetail(m, (d) => {
        const g = seg(d, s);
        if (g) Object.assign(g, { text, edited: true, words: [] });
        return null;
      }),
    setSegmentSpeaker: (m, s, speaker) =>
      withDetail(m, (d) => {
        const g = seg(d, s);
        if (g) g.speakerGid = speaker;
        return null;
      }),
    // An empty name goes back to "Speaker N". The "Name your speakers" cards use other gids: a no-op for those.
    renameMeetingSpeaker: (m, speaker, name) => {
      if (!row(m)) return fail(`meeting not found: ${m}`);
      const sp = detailOf(m).speakers.find((x) => x.gid === speaker);
      if (sp) sp.name = name.trim() || null;
      return ok(null);
    },
    // Same rules and error codes as ghi-app's speakers_cmd; the lines follow their speakers.
    mergeMeetingSpeakers: (m, from, into) => {
      if (!row(m)) return fail(`meeting not found: ${m}`);
      const d = detailOf(m);
      const f = d.speakers.find((x) => x.gid === from);
      const t = d.speakers.find((x) => x.gid === into);
      if (!f || !t) return fail("notASpeaker");
      if (f === t) return fail("sameSpeaker");
      // A desktop call has a far side: only the mic speaker can be Me, so Me
      // and a far-side speaker never merge either way.
      if ((f.isMe || t.isMe) && row(m)?.mode === "call") return fail("farSide");
      for (const g of d.transcript.segments) if (g.speakerGid === from) g.speakerGid = into;
      for (const a of d.notes.actionItems) if (a.ownerSpeakerGid === from) a.ownerSpeakerGid = into;
      t.lines += f.lines;
      if (f.isMe) Object.assign(t, { isMe: true, notPerson: false });
      d.speakers = d.speakers.filter((x) => x !== f);
      return ok(null);
    },
    splitMeetingSpeaker: (m, speaker, segmentGids, fromSegment) => {
      if (!row(m)) return fail(`meeting not found: ${m}`);
      const d = detailOf(m);
      const sp = d.speakers.find((x) => x.gid === speaker);
      if (!sp) return fail("notASpeaker");
      const own = d.transcript.segments.filter((g) => g.speakerGid === speaker);
      const from = own.find((g) => g.gid === fromSegment);
      if ((segmentGids.length > 0) === (fromSegment != null) || (fromSegment != null && !from)) return fail("nothingToSplit");
      const moving = from ? own.filter((g) => (g.t0Ms ?? 0) >= (from.t0Ms ?? 0)) : segmentGids.map((id) => own.find((g) => g.gid === id));
      if (moving.some((g) => !g) || new Set(moving).size !== moving.length) return fail("nothingToSplit");
      if (moving.length >= own.length) return fail("wholeSpeaker");
      const slot = [1, 2, 3, 4, 5, 6, 7, 8].find((n) => !d.speakers.some((x) => x.colorSlot === n)) ?? 0;
      const created: MeetingSpeaker = {
        gid: gid("spk"),
        name: null,
        number: Math.max(...d.speakers.map((x) => x.number)) + 1,
        colorSlot: slot,
        isMe: false,
        notPerson: sp.notPerson,
        lines: moving.length,
        sampleT0Ms: null,
        sampleT1Ms: null,
        suggestion: null,
      };
      for (const g of moving) if (g) g.speakerGid = created.gid;
      sp.lines -= moving.length;
      d.speakers.push(created);
      return ok(created.gid);
    },
    setSpeakerNotPerson: (m, speaker, notPerson) => {
      if (!row(m)) return fail(`meeting not found: ${m}`);
      const sp = detailOf(m).speakers.find((x) => x.gid === speaker);
      if (!sp) return fail("notASpeaker");
      if (notPerson && sp.isMe) return fail("isMe");
      sp.notPerson = notPerson;
      if (notPerson) sp.suggestion = null;
      return ok(null);
    },
    updateNoteBlock: (m, b, text) =>
      withDetail(m, (d) => {
        const x = block(d, b);
        if (x) Object.assign(x, { text, origin: x.origin === "ai" ? "aiEdited" : x.origin });
        return null;
      }),
    addNoteBlock: (m, text) =>
      withDetail(m, (d) => {
        const x: NoteBlockView = { gid: gid("blk"), kind: "note", origin: "user", text, pinned: false, citations: [] };
        d.notes.blocks.push(x);
        return x;
      }),
    deleteNoteBlock: (m, b) =>
      withDetail(m, (d) => {
        d.notes.blocks = d.notes.blocks.filter((x) => x.gid !== b && x.kind !== `enhanced:${b}`);
        return null;
      }),
    addActionItem: (m, text, owner) =>
      withDetail(m, (d) => {
        const x: ActionItemView = { gid: gid("act"), text, ownerSpeakerGid: owner, dueText: null, done: false, origin: "user", citations: [] };
        d.notes.actionItems.push(x);
        return x;
      }),
    updateActionItem: (m, a, text) =>
      withDetail(m, (d) => {
        const x = item(d, a);
        if (x) Object.assign(x, { text, origin: x.origin === "ai" ? "aiEdited" : x.origin });
        return null;
      }),
    setActionDone: (m, a, done) =>
      withDetail(m, (d) => {
        const x = item(d, a);
        if (x) x.done = done;
        return null;
      }),
    setActionOwner: (m, a, owner) =>
      withDetail(m, (d) => {
        const x = item(d, a);
        if (x) Object.assign(x, { ownerSpeakerGid: owner, origin: x.origin === "ai" ? "aiEdited" : x.origin });
        return null;
      }),
    deleteActionItem: (m, a) =>
      withDetail(m, (d) => {
        d.notes.actionItems = d.notes.actionItems.filter((x) => x.gid !== a);
        return null;
      }),
    listTemplates: () => Promise.resolve(TEMPLATES),
    regenerateNotes: (m, template) => {
      const r = row(m);
      if (!r) return fail(`meeting not found: ${m}`);
      if (template) r.template = template;
      // Keep what the user wrote or edited; rewrite the rest.
      const d = detailOf(m);
      const fresh = build(document.documentElement.lang === "vi" ? "vi" : "en");
      const keep = d.notes.blocks.filter((b) => b.origin !== "ai");
      d.notes = {
        blocks: [...fresh.notes.blocks.filter((b) => b.origin === "ai" && !b.kind.startsWith("enhanced:")), ...keep],
        actionItems: [...d.notes.actionItems.filter((a) => a.origin !== "ai" || a.done), ...fresh.notes.actionItems.filter((a) => a.origin === "ai")],
        sections: TEMPLATES.find((t) => t.id === template)?.sections ?? [],
      };
      host.process(m);
      return ok(false);
    },
    retranscribe: (m, language) => {
      const r = row(m);
      if (!r) return fail(`meeting not found: ${m}`);
      if (sensitiveMeetings.has(m) || audioDeleted.has(m)) return fail("the meeting's audio is no longer kept");
      languages.set(m, language === "auto" ? null : language);
      host.process(m);
      return ok(false);
    },
    searchMeetings: (req) => {
      const q = req.text.trim();
      if (!q) return ok({ hits: [], truncated: false });
      const hits: SearchHitView[] = [];
      for (const r of host.rows) {
        if (req.meeting && r.gid !== req.meeting) continue;
        if (req.source && r.source !== req.source) continue;
        if (req.fromMs != null && (r.startedAt ?? 0) < req.fromMs) continue;
        if (req.toMs != null && (r.startedAt ?? 0) > req.toMs) continue;
        const d = detailOf(r.gid);
        const add = (kind: string, item: string, text: string, s: SegmentView | null) => {
          const h = highlights(text, q);
          if (h.length) {
            hits.push({
              kind,
              meeting: r.gid,
              meetingTitle: r.title,
              meetingStartedAt: r.startedAt,
              item,
              speakerGid: s?.speakerGid ?? null,
              t0Ms: s?.t0Ms ?? null,
              t1Ms: s?.t1Ms ?? null,
              snippet: text,
              highlights: h,
              exact: text.toLowerCase().includes(q.toLowerCase()),
            });
          }
        };
        add("note", `title-${r.gid}`, r.title, null);
        d.transcript.segments.forEach((s) => add("segment", s.gid, s.text, s));
        d.notes.blocks.forEach((b) => add("note", b.gid, b.text, null));
      }
      hits.sort((a, b) => Number(b.exact) - Number(a.exact));
      return ok({ hits: hits.slice(req.offset, req.offset + req.limit), truncated: false });
    },
    issueAudioPlay: (m) => {
      if (!row(m)) return fail(`meeting not found: ${m}`);
      const token = `mock-play-${m}`;
      if (!audio.has(token)) audio.set(token, silentWav(sample.durationSeconds * 1000));
      return ok({ token, durationMs: sample.durationSeconds * 1000 });
    },
    waveformPeaks: (m) =>
      withDetail(m, (d) => {
        const n = sample.durationSeconds * 10;
        const peaks = Array.from({ length: n }, (_, i) => {
          const t = i * 100;
          const speaking = d.transcript.segments.some((s) => (s.t0Ms ?? 0) <= t && (s.t1Ms ?? 0) > t);
          return speaking ? 150 + Math.round(80 * Math.abs(Math.sin(i / 7))) : 20;
        });
        return { perSecond: 10, peaks };
      }),
    exportMeeting: (m, format) => (row(m) ? ok(`${row(m)!.title || "Meeting"}.${format === "markdown" ? "md" : format === "text" ? "txt" : format}`) : fail("not found")),
    exportMeetings: (ms) => ok(ms.length),
    exportObsidian: (m) => (row(m) ? ok(`${row(m)!.title || "Meeting"}.md`) : fail("not found")),
    meetingAsText: (m, markdown) =>
      withDetail(m, (d) => {
        const title = row(m)?.title ?? "";
        const lines = d.notes.blocks.filter((b) => b.kind === "tldr").map((b) => (markdown ? `- ${b.text}` : `• ${b.text}`));
        return [markdown ? `# ${title}` : title, "", ...lines].join("\n");
      }),
    revealLastExport: () => ok(null),
    exportDestination: () => ok(exportFolder),
    chooseExportFolder: () => ok((exportFolder = "Meeting notes")),
    obsidianVault: () => ok(obsidianVault),
    chooseObsidianVault: () => ok((obsidianVault = "Notes vault")),
    pickImportFiles: () => ok(importGroup() ? stageZoomGroup() : stageSamples()),
    stagedFiles: (ids) => Promise.resolve(ids.map((id) => staged.get(id)).filter((f): f is StagedFile => !!f)),
    takeDroppedFiles: () => {
      const ids = dropped;
      dropped = [];
      return Promise.resolve(ids.map((id) => staged.get(id)).filter((f): f is StagedFile => !!f));
    },
    unstageFiles: (ids) => {
      ids.forEach((id) => staged.delete(id));
      return Promise.resolve(refreshSuperseded());
    },
    importTracksSeparately: (group) => {
      const changed: StagedFile[] = [];
      for (const f of staged.values()) {
        if (f.group !== group) continue;
        f.group = null;
        f.participant = null;
        f.problems = f.problems.filter((p) => p !== "duplicate" && p !== "tooManyTracks");
        f.duplicateOf = null;
        changed.push({ ...f });
      }
      return Promise.resolve({ status: "ok" as const, data: [...changed, ...refreshSuperseded()] });
    },
    startImport: (ids) => {
      // The tracks of a group are one import, under the group's id.
      const files = ids.map((id) => staged.get(id)).filter((f): f is StagedFile => !!f);
      ids.forEach((id) => staged.delete(id));
      const jobs = new Map<string, StagedFile[]>();
      const blocked = (f: StagedFile) => ["unsupported", "empty", "superseded", "tooManyTracks"].some((p) => f.problems.includes(p as never));
      const usable = files.filter((f) => !blocked(f));
      const size = (g: string) => usable.filter((f) => f.group === g).length;
      for (const f of usable) {
        // A group left with one track is a file, under the file's id (as in the core).
        const key = f.group && size(f.group) >= 2 ? f.group : f.id;
        jobs.set(key, [...(jobs.get(key) ?? []), f]);
      }
      [...jobs.entries()].forEach(([id, members], k) => {
        const f = members[0]!;
        const group = members.length > 1;
        const at = (ms: number, fn: () => void) => window.setTimeout(fn, 1200 * k + ms);
        emitUpdate({ id, state: "queued", meeting: null, progress: 0, error: null });
        const meeting = `import-${id}`;
        [0.2, 0.55, 1].forEach((p, i) =>
          at(300 * (i + 1), () => {
            if (cancelled.has(id)) return;
            emitUpdate({ id, state: "decoding", meeting, progress: p, error: null });
          }),
        );
        at(1100, () => {
          if (cancelled.has(id)) return emitUpdate({ id, state: "cancelled", meeting: null, progress: 0, error: null });
          host.rows.unshift({
            gid: meeting,
            title: group ? (f.title ?? "Zoom recording") : f.name.replace(/\.[^.]+$/, ""),
            startedAt: f.startedAt ?? Date.now(),
            durationMs: f.durationMs ?? 0,
            source: "file",
            mode: f.channels > 1 && !group ? "call" : "room",
            status: "processing",
            transcriptVersion: 0,
            cloudUsed: false,
            consentConfirmed: false,
            sensitive: false,
            template: null,
            people: group ? members.flatMap((m, i) => (m.participant ? [{ name: m.participant, colorSlot: [1, 2, 4, 8][i % 4]!, isMe: false }] : [])) : [],
            job: { kind: "final_pass", progress: 0, waitingForModels: false },
            folder: null,
            tags: [],
            sourceApp: group ? "zoom" : null,
            summary: null,
            unnamedVoices: 0,
          });
          emitUpdate({ id, state: "done", meeting, progress: 1, error: null });
          host.process(meeting);
        });
      });
      return ok(null);
    },
    cancelImport: (id) => {
      cancelled.add(id);
      return Promise.resolve();
    },
  };
}
