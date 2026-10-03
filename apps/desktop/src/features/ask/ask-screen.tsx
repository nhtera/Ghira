// SPDX-License-Identifier: Apache-2.0
// D9 Ask across meetings: a scope, a visual thread of questions and answers,
// and a field. Each question is independent (the core keeps no conversation);
// the answer runs on the local model and links to the moments it came from.
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { APP_NAME, formatClock } from "@ghi/i18n";
import { Button, EmptyState, Icon, Menu, cn, usePlatform, type MenuItem } from "@ghi/ui";
import type { AskAllAnswer, NotesLanguage } from "../../bindings";
import { ipc } from "../../ipc";
import { useLlmName } from "../meeting/llm-name";
import { useMeetings } from "../library/use-meetings";
import { personName } from "../people/person-label";
import { usePeople } from "../people/queries";
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

function AnswerCard({ entry, model, onOpen, onSearch }: { entry: Entry; model: string; onOpen: (meeting: string, tMs: number | null) => void; onSearch: (q: string) => void }) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const a = entry.answer;
  return (
    <li data-testid="ask-entry" className="flex flex-col gap-2.5">
      <p className="m-0 max-w-[80%] self-end rounded-[14px_14px_4px_14px] bg-sunk px-3.5 py-2.5 text-[14.5px]">{entry.question}</p>
      {entry.state === "thinking" && (
        <p className="text-small m-0 flex items-center gap-1.5 text-muted">
          <Icon name="progress_activity" size={17} className="animate-spin motion-reduce:animate-none" />
          <Thinking since={entry.startedAt} />
        </p>
      )}
      {entry.state === "error" && <AskError error={entry.error ?? ""} />}
      {entry.state === "done" && a && !a.answered && (
        <div data-testid="ask-not-discussed" className="flex gap-3 rounded-panel bg-surface2 px-4 py-3.5">
          <Icon name="search_off" size={22} className="flex-none text-muted" />
          <div className="flex min-w-0 flex-col gap-1.5">
            <b className="text-[14.5px] font-semibold">{t("ask.notDiscussed.title")}</b>
            <p className="text-small m-0 leading-normal text-muted">
              {t("ask.notDiscussed.found", { count: a.sources.length, app: APP_NAME })}
              {a.searched.length > 0 && ` ${t("ask.meeting.searched")} ${a.searched.join(", ")}`}
            </p>
            <Button size="sm" className="self-start" onClick={() => onSearch(a.searched[0] ?? entry.question)}>
              {t("ask.notDiscussed.search", { query: entry.question })}
            </Button>
          </div>
        </div>
      )}
      {entry.state === "done" && a?.answered && (
        // Text nodes only: the answer is model output (RT-6). The core returns the citations beside the text, so the chips follow it.
        <p className="font-serif m-0 text-[17px] leading-[1.75] text-pretty whitespace-pre-wrap">
          {a.text}
          {a.citations.map((c, i) => {
            const { missing, t0Ms } = c.citation;
            const playable = t0Ms != null && !missing;
            const time = t0Ms != null ? formatClock(t0Ms, { pad: true }) : null;
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
                  "text-mono mr-0.5 ml-[5px] inline-flex h-6 max-w-full items-center gap-0.5 rounded-seg border bg-surface2 pr-[7px] pl-1 align-[2px] text-[12px] whitespace-normal text-muted",
                  "hover:border-accent hover:text-accent",
                  missing ? "border-dashed border-line2" : "border-ctl",
                )}
              >
                <Icon name={missing ? "link_off" : "play_arrow"} size={14} />
                <span className="truncate">{time && !missing ? `${c.meeting.title} · ${time}` : c.meeting.title}</span>
              </button>
            );
          })}
        </p>
      )}
      {entry.state === "done" && a && a.sources.length > 0 && (
        <p className="m-0 flex items-center gap-1.5 text-[12px] text-muted">
          <Icon name="lock" size={15} />
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

/** The design's scope switch: a sunk pill of radios (arrow keys move the choice); a person or date range is picked from a menu button beside the chosen radio. */
function ScopePills({ value, onChange, options }: { value: ScopeKind; onChange: (k: ScopeKind) => void; options: { value: ScopeKind; label: string; menu?: { label: string; trigger: string | null; items: MenuItem[]; onOpen?: () => void } }[] }) {
  const { t } = useTranslation();
  const pill = "inline-flex h-7 items-center gap-0.5 rounded-[7px] px-2.5 text-[12.5px] font-medium whitespace-nowrap";
  const chosen = "bg-surface text-ink shadow-[0_0_0_1px_var(--line2)]";
  const step = (e: KeyboardEvent<HTMLDivElement>) => {
    const d = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    const next = options[(options.findIndex((o) => o.value === value) + d + options.length) % options.length]!;
    onChange(next.value);
    e.currentTarget.querySelector<HTMLElement>(`[data-scope="${next.value}"]`)?.focus();
  };
  const menu = options.find((o) => o.value === value)?.menu;
  return (
    <div className="inline-flex items-center gap-0.5 rounded-[9px] bg-sunk p-0.5">
      <div role="radiogroup" aria-label={t("ask.scopeLabel")} onKeyDown={step} className="inline-flex gap-0.5">
        {options.map((o) => {
          const on = o.value === value;
          return (
            <button key={o.value} type="button" role="radio" data-scope={o.value} aria-checked={on} tabIndex={on ? 0 : -1} onClick={() => onChange(o.value)} className={cn(pill, on ? chosen : "text-muted hover:text-ink")}>
              {o.label}
              {o.menu && <Icon name="expand_more" size={15} />}
            </button>
          );
        })}
      </div>
      {menu && (
        <Menu
          label={menu.label}
          align="start"
          trigger={
            <button type="button" onClick={menu.onOpen} className={cn(pill, "text-ink hover:bg-surface")}>
              {menu.trigger ?? menu.label}
              <Icon name="expand_more" size={15} />
            </button>
          }
          items={menu.items}
        />
      )}
    </div>
  );
}

/** With `title` the body draws its own header (title, privacy pill, New question) so the actions can follow the thread. */
export function AskScreenBody({ meeting, title, subtitle }: { meeting?: string; title?: string; subtitle?: string }) {
  const { t, i18n } = useTranslation();
  const platform = usePlatform();
  const navigate = useNavigate();
  const meetings = useMeetings();
  const model = useLlmName() ?? "LLM";
  const language: NotesLanguage = i18n.language.startsWith("vi") ? "vi" : "en";
  // Recomputed when the range menu opens and on every question (not frozen at mount).
  const [now, setNow] = useState(() => new Date());

  const [kind, setKind] = useState<ScopeKind>(meeting ? "meeting" : "all");
  const [range, setRange] = useState<RangeKey>("last30Days");
  const [personGid, setPersonGid] = useState<string | null>(null);
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
  // People who were in at least one meeting; the picker only offers them.
  const people = usePeople().data?.people.filter((p) => p.meetings > 0) ?? [];
  // Nobody picked (or the picked person was merged away): the picker asks again, it never falls back to someone else.
  const person = people.find((p) => p.gid === personGid);
  const needsPerson = kind === "person" && !person;
  const meetingTitle = rows.find((r) => r.gid === meeting)?.title;
  const scopeName = (k: ScopeKind, r: RangeKey) =>
    k === "person" && person ? personName(person, t) : k === "meeting" ? (meetingTitle ?? t("ask.scopes.thisMeeting")) : k === "range" ? t(`ask.rangeShort.${r}`) : t("ask.scopes.allMeetings");

  const ask = async (text: string) => {
    const q = text.trim();
    if (!q || inFlight.current || needsPerson) return;
    inFlight.current = true;
    setQuestion("");
    inputRef.current?.focus();
    const at = new Date();
    setNow(at);
    const id = ++nextId.current;
    setEntries((es) => [...es, { id, question: q, startedAt: Date.now(), state: "thinking", scopeLabel: scopeName(kind, range) }]);
    try {
      const r = await ipc.commands.askAllMeetings(q, buildScope(kind, meeting, range, at, person?.gid), language);
      if (r.status === "error") patch(id, { state: "error", error: r.error });
      else patch(id, { state: "done", answer: r.data });
    } finally {
      inFlight.current = false;
    }
  };

  const searchableRows = rows.filter(searchable);
  // Only the meetings the core reads: the person's own count also holds ones still being processed.
  const personCount = person ? (person.isMe ? searchableRows.length : searchableRows.filter((r) => r.people.some((x) => x.name === person.name)).length) : 0;
  const rangeLabel = (k: RangeKey) => t(`ask.ranges.${k}`, { count: countInRange(rows, k, now) });
  const scopeLine =
    kind === "meeting"
      ? meetingTitle
        ? t("ask.scopeLine.thisMeeting", { title: meetingTitle })
        : null
      : kind === "person"
        ? person
          ? t("ask.scopeLine.person", { count: personCount, name: personName(person, t) })
          : null
        : kind === "range"
        ? t("ask.scopeLine.dateRange", { range: rangeLabel(range) })
        : meetings.isSuccess
          ? t("ask.scopeLine.allMeetings", { count: searchableRows.length })
          : null;

  const options = [
    { value: "all" as const, label: t("ask.scopes.allMeetings") },
    ...(meeting ? [{ value: "meeting" as const, label: t("ask.scopes.thisMeeting") }] : []),
    ...(people.length > 0
      ? [
          {
            value: "person" as const,
            label: t("ask.scopes.person"),
            menu: { label: t("ask.personMenu"), trigger: person ? personName(person, t) : null, items: people.map((p) => ({ label: personName(p, t), onSelect: () => setPersonGid(p.gid) })) },
          },
        ]
      : []),
    {
      value: "range" as const,
      label: t("ask.scopes.dateRange"),
      menu: { label: t("ask.rangeMenu"), trigger: t(`ask.rangeShort.${range}`), onOpen: () => setNow(new Date()), items: RANGES.map((k) => ({ label: rangeLabel(k), onSelect: () => setRange(k) })) },
    },
  ];
  const onDevice = (
    <span className="inline-flex h-[30px] items-center gap-1.5 rounded-pill border border-accent-soft bg-accent-soft px-3 pl-[9px] text-[12.5px] font-semibold text-accent">
      <Icon name="lock" size={16} />
      {t("ask.onDevice", { context: platform })}
    </span>
  );
  const newQuestion = entries.length > 0 && (
    <Button size="sm" icon="add" disabled={thinking} onClick={() => (setEntries([]), inputRef.current?.focus())}>
      {t("ask.newQuestion")}
    </Button>
  );

  if (meetings.isSuccess && rows.length === 0) return <EmptyState kind="ask" className="mt-10" />;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div data-tauri-drag-region className="flex flex-none flex-col gap-3 border-b border-line px-7 pt-5 pb-3.5">
        <div className="flex items-center gap-2.5">
          {title ? (
            <div data-tauri-drag-region className="min-w-0 flex-1">
              <h1 className="text-title m-0 truncate">{title}</h1>
              {subtitle && <p className="m-0 text-[13px] text-muted">{subtitle}</p>}
            </div>
          ) : (
            <span className="flex-1" />
          )}
          {onDevice}
          {newQuestion}
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <ScopePills value={kind} onChange={(k) => (setNow(new Date()), setKind(k))} options={options} />
          {scopeLine && (
            <p data-testid="ask-scope-line" className="m-0 flex items-center gap-1.5 text-[12.5px] text-muted">
              <Icon name="manage_search" size={16} />
              {scopeLine}
            </p>
          )}
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-7 py-6">
        <div className="flex max-w-[720px] flex-col gap-7">
          {entries.length === 0 ? (
            meetings.isSuccess && (
              <div className="flex flex-col gap-2">
                <p className="m-0 text-[11.5px] font-semibold tracking-[0.06em] text-faint uppercase">{t("ask.suggested")}</p>
                {SUGGESTIONS.map((k) => (
                  <button
                    key={k}
                    type="button"
                    onClick={() => void ask(t(`ask.suggestions.${k}`))}
                    className="flex min-h-[46px] items-center gap-2.5 rounded-row border border-ctl bg-surface px-3.5 py-2 text-left text-[14px] hover:border-accent"
                  >
                    <Icon name="forum" size={18} className="text-accent" />
                    <span className="flex-1">{t(`ask.suggestions.${k}`)}</span>
                    <Icon name="arrow_forward" size={18} className="text-faint" />
                  </button>
                ))}
              </div>
            )
          ) : (
            <ol aria-label={t("ask.thread")} aria-live="polite" className="m-0 flex list-none flex-col gap-7 p-0">
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
      </div>
      <div className="flex-none border-t border-line px-7 pt-3 pb-[18px]">
        <div className="flex h-[46px] max-w-[720px] items-center gap-2 rounded-xl border border-line2 bg-surface pr-1.5 pl-3.5 focus-within:outline-2 focus-within:outline-accent">
          <Icon name="forum" size={19} className="text-faint" />
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
            className="text-body min-w-0 flex-1 border-0 bg-transparent text-[14.5px] text-ink outline-none"
          />
          <Button variant="primary" icon="arrow_upward" aria-label={t("ask.meeting.send")} className="size-[34px] rounded-[9px]" disabled={!question.trim() || thinking || needsPerson} onClick={() => void ask(question)} />
        </div>
      </div>
    </div>
  );
}
