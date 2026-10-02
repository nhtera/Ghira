// SPDX-License-Identifier: Apache-2.0
// D9 Ask across meetings: a scope, a visual thread of questions and answers,
// and a field. Each question is independent (the core keeps no conversation);
// the answer runs on the local model and links to the moments it came from.
import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { formatClock } from "@ghi/i18n";
import { Button, EmptyState, Icon, Menu, Segmented, cn, usePlatform } from "@ghi/ui";
import type { AskAllAnswer, NotesLanguage } from "../../bindings";
import { ipc } from "../../ipc";
import { useMeetings } from "../library/use-meetings";
import { AskError } from "./ask-error";
import { RANGES, buildScope, countInRange, searchable, type RangeKey, type ScopeKind } from "./scope";

const SUGGESTIONS = ["decided", "actions", "themes"] as const;

type Entry = {
  id: number;
  question: string;
  startedAt: number;
  state: "thinking" | "done" | "error";
  /** What was searched (shown in this answer's footer; the scope can change later). */
  scopeLabel: string;
  answer?: AskAllAnswer;
  error?: string;
};

/** The counter is for the eyes only: a live region that ticks every half second is noise. */
function Thinking({ since }: { since: number }) {
  const { t } = useTranslation();
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 500);
    return () => window.clearInterval(timer);
  }, []);
  return (
    <>
      <span className="sr-only">{t("ask.thinking")}</span>
      <span aria-hidden>{t("ask.meeting.thinking", { seconds: Math.max(0, Math.floor((now - since) / 1000)) })}</span>
    </>
  );
}

function useLlmName() {
  const q = useQuery({
    queryKey: ["models-status"],
    queryFn: async () => {
      const r = await ipc.commands.modelsStatus();
      return r.status === "ok" ? r.data : null;
    },
  });
  return q.data?.models.find((m) => m.role === "llm" && m.installed)?.id ?? "LLM";
}

function AnswerCard({ entry, model, onOpen, onSearch }: { entry: Entry; model: string; onOpen: (meeting: string, tMs: number | null) => void; onSearch: (q: string) => void }) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const a = entry.answer;
  return (
    <li data-testid="ask-entry" className="flex flex-col gap-2">
      <p className="text-body m-0 self-end rounded-panel bg-sunk px-3 py-1.5 font-semibold">{entry.question}</p>
      {entry.state === "thinking" && (
        <p className="text-small m-0 flex items-center gap-2 text-muted">
          <Icon name="progress_activity" size={16} className="animate-spin motion-reduce:animate-none" />
          <Thinking since={entry.startedAt} />
        </p>
      )}
      {entry.state === "error" && <AskError error={entry.error ?? ""} />}
      {entry.state === "done" && a && !a.answered && (
        <div data-testid="ask-not-discussed" className="flex flex-col gap-1.5 rounded-panel border border-dashed border-line2 bg-surface p-3">
          <b className="text-body flex items-center gap-1.5 font-semibold">
            <Icon name="search_off" size={18} className="text-muted" />
            {t("ask.notDiscussed.title")}
          </b>
          {a.searched.length > 0 && (
            <p className="text-small m-0 text-muted">
              {t("ask.meeting.searched")} {a.searched.join(", ")}
            </p>
          )}
          <Button size="sm" icon="search" className="self-start" onClick={() => onSearch(a.searched[0] ?? entry.question)}>
            {t("ask.notDiscussed.search", { query: entry.question })}
          </Button>
        </div>
      )}
      {entry.state === "done" && a?.answered && (
        <div className="flex flex-col gap-2 rounded-panel border border-line2 bg-surface p-3">
          {/* Text node only: the answer is model output (RT-6). */}
          <p className="text-body m-0 whitespace-pre-wrap">{a.text}</p>
          {a.citations.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {a.citations.map((c, i) => {
                const { missing, t0Ms } = c.citation;
                const playable = t0Ms != null && !missing;
                const time = t0Ms != null ? formatClock(t0Ms) : null;
                const name = time ? t("ask.openAt", { title: c.meeting.title, time }) : t("ask.openMeeting", { title: c.meeting.title });
                return (
                  <button
                    key={i}
                    type="button"
                    data-testid="ask-chip"
                    aria-label={missing ? `${name} · ${t("citation.missing")}` : name}
                    title={missing ? t("citation.missing") : undefined}
                    onClick={() => onOpen(c.meeting.meeting, playable ? t0Ms : null)}
                    className={cn(
                      "text-mono inline-flex h-6 max-w-full items-center gap-0.5 rounded-seg border bg-surface2 pr-[7px] pl-1 text-[12px] text-muted",
                      "hover:border-accent hover:text-accent",
                      missing ? "border-dashed border-line2" : "border-line2",
                    )}
                  >
                    <Icon name={missing ? "link_off" : "play_arrow"} size={14} />
                    <span className="truncate">{time && !missing ? `${c.meeting.title} · ${time}` : c.meeting.title}</span>
                  </button>
                );
              })}
            </div>
          )}
        </div>
      )}
      {entry.state === "done" && a?.answered && (
        <p className="text-small m-0 flex items-center gap-1.5 text-muted">
          <Icon name="lock" size={14} className="text-accent" />
          {t("ask.answeredLocal", { context: platform, model, count: a.sources.length })} · {entry.scopeLabel}
        </p>
      )}
      {entry.state === "done" && a && !a.semantic && (
        <p data-testid="ask-keyword-only" className="text-small m-0 text-muted">
          {t("ask.keywordOnly")}
        </p>
      )}
    </li>
  );
}

export function AskScreenBody({ meeting }: { meeting?: string }) {
  const { t, i18n } = useTranslation();
  const platform = usePlatform();
  const navigate = useNavigate();
  const meetings = useMeetings();
  const model = useLlmName();
  const language: NotesLanguage = i18n.language.startsWith("vi") ? "vi" : "en";
  // Recomputed when the range menu opens and on every question (not frozen at mount).
  const [now, setNow] = useState(() => new Date());

  const [kind, setKind] = useState<ScopeKind>(meeting ? "meeting" : "all");
  const [range, setRange] = useState<RangeKey>("last30Days");
  const [question, setQuestion] = useState("");
  const [entries, setEntries] = useState<Entry[]>([]);
  const nextId = useRef(0);
  const logEnd = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  // The core can't cancel a question: one at a time until it settles.
  const inFlight = useRef(false);
  const thinking = entries.some((e) => e.state === "thinking");

  useEffect(() => {
    logEnd.current?.scrollIntoView?.({ block: "end" });
  }, [entries]);

  const patch = (id: number, p: Partial<Entry>) => setEntries((es) => es.map((e) => (e.id === id ? { ...e, ...p } : e)));

  const rows = meetings.rows;
  const meetingTitle = rows.find((r) => r.gid === meeting)?.title;
  const scopeName = (k: ScopeKind, r: RangeKey) =>
    k === "meeting" ? (meetingTitle ?? t("ask.scopes.thisMeeting")) : k === "range" ? t(`ask.rangeShort.${r}`) : t("ask.scopes.allMeetings");

  const ask = async (text: string) => {
    const q = text.trim();
    if (!q || inFlight.current) return;
    inFlight.current = true;
    setQuestion("");
    inputRef.current?.focus();
    const at = new Date();
    setNow(at);
    const id = ++nextId.current;
    setEntries((es) => [...es, { id, question: q, startedAt: Date.now(), state: "thinking", scopeLabel: scopeName(kind, range) }]);
    try {
      const r = await ipc.commands.askAllMeetings(q, buildScope(kind, meeting, range, at), language);
      if (r.status === "error") patch(id, { state: "error", error: r.error });
      else patch(id, { state: "done", answer: r.data });
    } finally {
      inFlight.current = false;
    }
  };

  const searchableRows = rows.filter(searchable);
  const rangeLabel = (k: RangeKey) => t(`ask.ranges.${k}`, { count: countInRange(rows, k, now) });
  const scopeLine =
    kind === "meeting"
      ? meetingTitle
        ? t("ask.scopeLine.thisMeeting", { title: meetingTitle })
        : null
      : kind === "range"
        ? t("ask.scopeLine.dateRange", { range: rangeLabel(range) })
        : meetings.isSuccess
          ? t("ask.scopeLine.allMeetings", { count: searchableRows.length })
          : null;

  const options: { value: ScopeKind; label: string }[] = [
    { value: "all", label: t("ask.scopes.allMeetings") },
    ...(meeting ? [{ value: "meeting" as const, label: t("ask.scopes.thisMeeting") }] : []),
    { value: "range", label: t("ask.scopes.dateRange") },
  ];

  if (meetings.isSuccess && rows.length === 0) return <EmptyState kind="ask" className="mt-10" />;

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <div className="flex flex-none flex-wrap items-center gap-2">
        <Segmented<ScopeKind> label={t("ask.scopeLabel")} value={kind} onChange={(k) => (setNow(new Date()), setKind(k))} options={options} />
        {kind === "range" && (
          <Menu
            label={t("ask.rangeMenu")}
            align="start"
            trigger={
              <Button size="sm" icon="expand_more" aria-label={t("ask.rangeMenu")} onClick={() => setNow(new Date())}>
                {rangeLabel(range)}
              </Button>
            }
            items={RANGES.map((k) => ({ label: rangeLabel(k), onSelect: () => setRange(k) }))}
          />
        )}
        <span className="text-small inline-flex items-center gap-1 rounded-seg bg-accent-soft px-2 py-0.5 text-accent">
          <Icon name="lock" size={14} />
          {t("ask.onDevice", { context: platform })}
        </span>
        {entries.length > 0 && (
          <Button size="sm" variant="ghost" icon="add" className="ml-auto" disabled={thinking} onClick={() => (setEntries([]), inputRef.current?.focus())}>
            {t("ask.newQuestion")}
          </Button>
        )}
      </div>
      {scopeLine && (
        <p data-testid="ask-scope-line" className="text-small m-0 -mt-1 text-muted">
          {scopeLine}
        </p>
      )}
      <div className="min-h-0 flex-1 overflow-auto">
        {entries.length === 0 ? (
          meetings.isSuccess && (
            <div className="flex flex-col items-start gap-2">
              <p className="text-small m-0 text-muted">{t("ask.suggested")}</p>
              {SUGGESTIONS.map((k) => (
                <Button key={k} icon="forum" onClick={() => void ask(t(`ask.suggestions.${k}`))}>
                  {t(`ask.suggestions.${k}`)}
                </Button>
              ))}
            </div>
          )
        ) : (
          <ol aria-label={t("ask.thread")} aria-live="polite" className="m-0 flex list-none flex-col gap-4 p-0">
            {entries.map((e) => (
              <AnswerCard
                key={e.id}
                entry={e}
                model={model}
                onOpen={(id, tMs) =>
                  void navigate({
                    to: "/meetings/$id/$tab",
                    params: { id, tab: tMs != null ? "transcript" : "notes" },
                    search: tMs != null ? { t: tMs } : {},
                  })
                }
                onSearch={(q) => void navigate({ to: "/meetings", search: { q } })}
              />
            ))}
          </ol>
        )}
        <div ref={logEnd} />
      </div>
      <div className="flex flex-none items-center gap-2 border-t border-line pt-3">
        <input
          ref={inputRef}
          value={question}
          aria-label={t("ask.meeting.label")}
          placeholder={entries.length ? t("ask.followUpPlaceholder") : t("ask.placeholder")}
          onChange={(e) => setQuestion(e.target.value)}
          // Enter confirms an IME candidate too; only a plain Enter sends.
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.nativeEvent.isComposing && e.keyCode !== 229) {
              e.preventDefault();
              void ask(question);
            }
          }}
          className="text-body h-9 min-w-0 flex-1 rounded-ctl border border-ctl bg-surface px-3 text-ink focus-visible:outline-2 focus-visible:outline-accent"
        />
        <Button variant="primary" icon="arrow_upward" aria-label={t("ask.meeting.send")} disabled={!question.trim() || thinking} onClick={() => void ask(question)} />
      </div>
    </div>
  );
}
