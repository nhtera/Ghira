// SPDX-License-Identifier: Apache-2.0
// The mock core's AI side (phase 11d): API keys (kept in memory, never
// returned), the cloud send preview / send with a request log, "Ask this
// meeting", the custom vocabulary, export-everything and delete-all. `?cloudfail=1`
// in the URL makes cloud sends fail (to see the local fallback).
import type { AskAnswer, CloudLogEntry, CloudPreview, MeetingRow, MeetingTranscript, Vocabulary } from "../bindings";
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
const MODELS = [
  { provider: "openai", model: "gpt-4.1-mini" },
  { provider: "openai", model: "gpt-4.1" },
  { provider: "anthropic", model: "claude-haiku-4-5" },
  { provider: "anthropic", model: "claude-sonnet-4-5" },
  { provider: "gemini", model: "gemini-2.5-flash" },
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
  | "vocabulary"
  | "setVocabulary"
  | "ignoreLearnedTerm"
  | "exportEverything"
  | "deleteAllData"
>;

/** Meetings marked "never send to cloud" on the mock. */
export const lockedMeetings = new Set<string>();

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
      if (lockedMeetings.has(meeting)) return fail("cloud AI is off for this meeting");
      const t = host.transcript(meeting);
      const names = ask.redact ? [...host.names(meeting), ...ask.extraNames] : [];
      let text = (t?.segments ?? []).map((s) => s.text).join("\n");
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
      if (!row(meeting)) return fail(`meeting not found: ${meeting}`);
      if (!question.trim()) return fail("ask a question");
      // A little thinking time, as on the real model.
      return new Promise((resolve) => window.setTimeout(() => resolve({ status: "ok", data: answer(host, meeting, question, "local") }), 600));
    },
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
    deleteAllData: () => {
      host.rows.splice(0, host.rows.length);
      return ok(null);
    },
  };
}
