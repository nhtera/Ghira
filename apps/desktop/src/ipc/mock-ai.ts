// SPDX-License-Identifier: Apache-2.0
// The mock core's AI side (phase 11d): API keys (kept in memory, never
// returned), the cloud send preview / send with a request log, "Ask this
// meeting", the custom vocabulary, export-everything and delete-all. `?cloudfail=1`
// in the URL makes cloud sends fail (to see the local fallback).
import type { AskAllAnswer, AskScope, AskAnswer, CloudLogEntry, CloudPreview, MeetingRow, MeetingRef, MeetingTranscript, RelatedHit, Vocabulary } from "../bindings";
import email from "@ghi/ui/mocks/email.json";
import type { Commands } from "./ipc";

type Result<T> = { status: "ok"; data: T } | { status: "error"; error: string };
const ok = <T>(data: T): Promise<Result<T>> => Promise.resolve({ status: "ok", data });
const fail = <T>(error: string): Promise<Result<T>> => Promise.resolve({ status: "error", error });

export interface AiHost {
  rows: MeetingRow[];
  process(meeting: string): void;
  transcript(meeting: string): MeetingTranscript | null;
  names(meeting: string): string[];
  strictOffline(): boolean;
}

const PROVIDERS = ["openai", "anthropic", "gemini"];
// The menus of crates/ghi-llm/prices.toml (each provider's first is its default).
const MODELS = [
  { provider: "openai", model: "gpt-6.1-sol" },
  { provider: "openai", model: "gpt-6-luna" },
  { provider: "openai", model: "gpt-6-astra" },
  { provider: "openai", model: "gpt-4.1-mini" },
  { provider: "anthropic", model: "claude-sonnet-5-5" },
  { provider: "anthropic", model: "claude-haiku-4-5" },
  { provider: "anthropic", model: "claude-opus-5-5" },
  { provider: "gemini", model: "gemini-3.8-flash" },
  { provider: "gemini", model: "gemini-3.5-flash-lite" },
  { provider: "gemini", model: "gemini-3.1-flash-lite" },
];
const keys = new Set<string>();
const log: CloudLogEntry[] = [];
type Pending = { meeting: string; provider: string; model: string; ask: string | null; tokens: number };
const pending = new Map<string, Pending>();
let seq = 0;
let terms: string[] = ["Nemotron", "CoreML", "Plaud"];
const ignored = new Set<string>();

const fold = (s: string) =>
  s
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[đĐ]/g, "d")
    .toLowerCase();

function answer(host: AiHost, meeting: string, question: string, engine: string): AskAnswer {
  const t = host.transcript(meeting);
  const words = fold(question)
    .split(/\W+/)
    .filter((w) => w.length > 3);
  const hit = t?.segments.find((s) => words.some((w) => fold(s.text).includes(w)));
  if (!hit) return { answered: false, text: "", citations: [], searched: words, engine };
  return {
    answered: true,
    text: hit.text,
    citations: [{ t0Ms: hit.t0Ms, t1Ms: hit.t1Ms, quote: hit.text, speakerGid: hit.speakerGid, stale: false, missing: false }],
    searched: [],
    engine,
  };
}

function vocabulary(host: AiHost): Vocabulary {
  const learned = [...new Set(host.rows.flatMap((r) => r.people.map((p) => p.name)))]
    .filter((n) => !ignored.has(n) && !terms.some((t) => fold(t) === fold(n)))
    .sort();
  return { terms: [...terms], learned, maxTerms: 200 };
}


const refOf = (r: MeetingRow): MeetingRef => ({ meeting: r.gid, title: r.title, startedAt: r.startedAt });
const inScope = (r: MeetingRow, scope: AskScope) =>
  (scope.meetings.length === 0 || scope.meetings.includes(r.gid)) &&
  // Person gids on the mock are `person-<name>` (see mock-people.ts); Me is in every meeting.
  (scope.persons.length === 0 || scope.persons.includes("person-me") || r.people.some((p) => scope.persons.includes(`person-${p.name.toLowerCase()}`))) &&
  // A meeting with no start time can't be placed in a date range.
  (scope.fromMs == null || (r.startedAt != null && r.startedAt >= scope.fromMs)) &&
  (scope.toMs == null || (r.startedAt != null && r.startedAt <= scope.toMs));

/** `?askfail=busyRecording|busyNotes|noModel`: the core refuses the way `local_model_free` does. */
const refusal = () => {
  const code = new URLSearchParams(location.search).get("askfail");
  return code && ["busyRecording", "busyNotes", "noModel"].includes(code) ? code : null;
};

/** Ask across meetings: a cited passage from up to two meetings; "pricing" is never discussed. */
function answerAll(host: AiHost, question: string, scope: AskScope): AskAllAnswer {
  const words = fold(question)
    .split(/\W+/)
    .filter((w) => w.length > 3);
  const semantic = !new URLSearchParams(location.search).has("keywordonly");
  const rows = host.rows.filter((r) => r.status === "ready" && inScope(r, scope));
  const sources = rows.slice(0, 3).map(refOf);
  if (words.includes("pricing")) return { answered: false, text: "", citations: [], searched: words, sources, semantic };
  const picks: { row: MeetingRow; seg: MeetingTranscript["segments"][number] }[] = [];
  for (const r of rows) {
    const seg = host.transcript(r.gid)?.segments.find((s) => words.some((w) => fold(s.text).includes(w)));
    if (seg) picks.push({ row: r, seg });
    if (picks.length === 2) break;
  }
  // Nothing matched by words: the model still found related passages by meaning.
  for (const r of semantic ? rows : []) {
    if (picks.length >= 2) break;
    const seg = host.transcript(r.gid)?.segments[0];
    if (seg && !picks.some((p) => p.row.gid === r.gid)) picks.push({ row: r, seg });
  }
  if (picks.length === 0) return { answered: false, text: "", citations: [], searched: words, sources, semantic };
  return {
    answered: true,
    // The sample meetings share a transcript: say a repeated passage once.
    text: [...new Set(picks.map((p) => p.seg.text))].join("\n"),
    citations: picks.map((p) => ({
      meeting: refOf(p.row),
      citation: { t0Ms: p.seg.t0Ms, t1Ms: p.seg.t1Ms, quote: p.seg.text, speakerGid: p.seg.speakerGid, stale: false, missing: false },
    })),
    searched: [],
    sources: picks.map((p) => refOf(p.row)),
    semantic,
  };
}

/** Passages by meaning, one per meeting (the UI leaves out meetings that already have keyword hits). */
function relatedTo(host: AiHost, text: string, scope: AskScope, limit: number): RelatedHit[] {
  if (fold(text.trim()).length < 3 || new URLSearchParams(location.search).has("keywordonly")) return [];
  const out: RelatedHit[] = [];
  for (const r of host.rows) {
    if (out.length >= Math.min(limit, 3)) break;
    if (r.status !== "ready" || !inScope(r, scope)) continue;
    const segs = host.transcript(r.gid)?.segments ?? [];
    const seg = segs[1] ?? segs[0];
    if (seg) out.push({ meeting: refOf(r), t0Ms: seg.t0Ms, t1Ms: seg.t1Ms, quote: seg.text });
  }
  return out;
}

type AiCommands = Pick<
  Commands,
  | "cloudKeys"
  | "setCloudKey"
  | "deleteCloudKey"
  | "cloudModels"
  | "cloudPreview"
  | "cloudSend"
  | "setMeetingCloudLocked"
  | "cloudRequestLog"
  | "askMeeting"
  | "askAllMeetings"
  | "relatedMeetings"
  | "vocabulary"
  | "setVocabulary"
  | "ignoreLearnedTerm"
  | "exportEverything"
  | "deleteAllData"
  | "draftFollowupEmail"
  | "meetingContacts"
  | "openMailDraft"
>;

/** Meetings marked "never send to cloud" on the mock. */
export const lockedMeetings = new Set<string>();

/** Meetings in sensitive mode on the mock: no audio, no cloud. */
export const sensitiveMeetings = new Set<string>();
/** Meetings whose audio sensitive mode deleted (turning it off does not bring it back). */
export const audioDeleted = new Set<string>();

/** A few lines of the transcript before and after redaction, preferring the lines redaction changed. */
function excerpt(raw: string, redacted: string): { excerptBefore: string | null; excerptAfter: string | null } {
  const a = raw.split("\n");
  const b = redacted.split("\n");
  const changed = a.flatMap((line, i) => (line !== b[i] ? [i] : []));
  const pick = (changed.length ? changed : a.map((_, i) => i)).slice(0, 4);
  if (!raw) return { excerptBefore: null, excerptAfter: null };
  return { excerptBefore: pick.map((i) => a[i]).join("\n"), excerptAfter: pick.map((i) => b[i]).join("\n") };
}

export function aiCommands(host: AiHost): AiCommands {
  const row = (m: string) => host.rows.find((r) => r.gid === m);
  return {
    cloudKeys: () => ok(PROVIDERS.map((provider) => ({ provider, stored: keys.has(provider) }))),
    setCloudKey: (provider, key) => {
      if (!PROVIDERS.includes(provider)) return fail(`unknown provider \`${provider}\``);
      if (!key.trim()) return fail("that doesn't look like an API key");
      keys.add(provider);
      return ok(null);
    },
    deleteCloudKey: (provider) => {
      keys.delete(provider);
      return ok(null);
    },
    cloudModels: () => Promise.resolve(MODELS),
    cloudPreview: (meeting, ask) => {
      if (host.strictOffline()) return fail("strict offline is on: nothing can be sent");
      if (!row(meeting)) return fail(`meeting not found: ${meeting}`);
      if (lockedMeetings.has(meeting) || sensitiveMeetings.has(meeting)) return fail("cloud AI is off for this meeting");
      const t = host.transcript(meeting);
      const names = ask.redact ? [...host.names(meeting), ...ask.extraNames] : [];
      const raw = (t?.segments ?? []).map((s) => s.text).join("\n");
      let text = raw;
      names.forEach((n, i) => (text = text.split(n).join(`[PERSON_${i + 1}]`)));
      const question = ask.task.kind === "ask" ? ask.task.question : null;
      if (question != null) {
        const a = answer(host, meeting, question, "local");
        if (!a.answered) return ok({ kind: "answer", ...a });
      }
      const body = JSON.stringify({ model: ask.model, max_tokens: 4096, messages: [{ role: "user", content: question ? `${question}\n\n${text}` : text }] }, null, 1);
      const tokens = Math.ceil(body.length / 3);
      const id = `plan-${++seq}`;
      pending.set(id, { meeting, provider: ask.provider, model: ask.model, ask: question, tokens });
      const preview: CloudPreview = {
        id,
        provider: ask.provider,
        model: ask.model,
        host: ask.provider === "anthropic" ? "api.anthropic.com" : ask.provider === "gemini" ? "generativelanguage.googleapis.com" : "api.openai.com",
        payload: body,
        sha256: Array.from({ length: 64 }, (_, i) => "0123456789abcdef"[(i * 7 + body.length) % 16]).join(""),
        tokensEst: tokens,
        costEstUsd: (tokens * 1 + 4096 * 5) / 1_000_000,
        retentionNote: "The provider may keep requests for up to 30 days for abuse monitoring.",
        warnings: [],
        redactions: names.length ? [{ kind: "person", count: names.length }] : [],
        ...excerpt(raw, text),
      };
      return ok({ kind: "preview", ...preview });
    },
    cloudSend: (id) => {
      const p = pending.get(id);
      pending.delete(id);
      if (!p) return fail("this preview expired: review it again");
      if (!keys.has(p.provider)) return fail(`no API key for ${p.provider}: add one in Settings → AI`);
      const r = row(p.meeting);
      log.unshift({ meeting: p.meeting, meetingTitle: r?.title ?? "", provider: p.provider, model: p.model, tokensIn: p.tokens, tokensOut: 800, at: Date.now() });
      if (r) r.cloudUsed = true;
      if (new URLSearchParams(location.search).has("cloudfail")) {
        if (!p.ask) host.process(p.meeting);
        return ok({ kind: "failed", reason: "The provider returned 503 (overloaded).", leftDevice: true });
      }
      if (p.ask != null) return ok({ kind: "answer", ...answer(host, p.meeting, p.ask, p.provider) });
      host.process(p.meeting);
      return ok({ kind: "notes" });
    },
    setMeetingCloudLocked: (meeting, locked) => {
      if (locked) lockedMeetings.add(meeting);
      else lockedMeetings.delete(meeting);
      return ok(null);
    },
    cloudRequestLog: (limit) => ok(log.slice(0, limit)),
    askMeeting: (meeting, question) => {
      if (refusal()) return fail(refusal()!);
      if (!row(meeting)) return fail(`meeting not found: ${meeting}`);
      if (!question.trim()) return fail("ask a question");
      // A little thinking time, as on the real model.
      return new Promise((resolve) => window.setTimeout(() => resolve({ status: "ok", data: answer(host, meeting, question, "local") }), 600));
    },
    askAllMeetings: (question, scope) => {
      if (refusal()) return fail(refusal()!);
      if (!question.trim()) return fail("ask a question");
      return new Promise((resolve) => window.setTimeout(() => resolve({ status: "ok", data: answerAll(host, question, scope) }), 600));
    },
    relatedMeetings: (text, scope, limit) => ok(relatedTo(host, text, scope, limit)),
    vocabulary: () => ok(vocabulary(host)),
    setVocabulary: (next) => {
      const out: string[] = [];
      for (const t of next.map((x) => x.trim()).filter(Boolean)) if (!out.some((o) => fold(o) === fold(t))) out.push(t);
      if (out.length > 200) return fail("at most 200 terms");
      terms = out;
      return ok(vocabulary(host));
    },
    ignoreLearnedTerm: (term) => {
      ignored.add(term);
      return ok(vocabulary(host));
    },
    exportEverything: (password) => (password.length < 8 ? fail("use at least 8 characters") : ok(`Ghira export ${new Date().toISOString().slice(0, 10)}.ghira`)),
    // Built from the design's sample draft, after a short "writing" pause.
    meetingContacts: () =>
      ok([
        { name: "Linh", email: "linh.tran@studio.vn" },
        { name: "Minh", email: "minh.nguyen@studio.vn" },
        { name: "Sarah", email: "sarah@studio.vn" },
      ]),
    openMailDraft: (_to, _subject, body) => ok({ truncated: body.length > 2000 }),
    draftFollowupEmail: (meeting, language, tone) => {
      if (refusal()) return fail(refusal()!);
      const r = row(meeting);
      if (!r) return fail(`meeting not found: ${meeting}`);
      const vi = language === "vi";
      const i = tone === "friendly" ? 0 : tone === "neutral" ? 1 : 2;
      const d = email.draft;
      const lang = vi ? "vi" : "en";
      const body = [d.open[lang][i], "", d.dec[lang], "", d.actH[lang], "- …", "", d.close[lang][i]].join("\n");
      return new Promise((resolve) => window.setTimeout(() => resolve({ status: "ok", data: { subject: d.subj[lang][i] ?? r.title, body } }), 500));
    },
    deleteAllData: () => {
      host.rows.splice(0, host.rows.length);
      return ok(null);
    },
  };
}
