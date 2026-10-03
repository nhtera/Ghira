// SPDX-License-Identifier: Apache-2.0
// The notepad (brief D4): plain lines typed during the call, each anchored to
// the meeting time it was typed at (invisible), with in-call tags Decision /
// Action / Question that feed the notes. Markdown-lite, rendered as text nodes.
import { formatClock } from "@ghi/i18n";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Fragment, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { Button, Icon, cn, usePlatform, useToast, type IconName } from "@ghi/ui";
import { useAppActions } from "../../shell/actions";
import { useLive } from "../../state/live";
import type { NoteKind, NoteLine } from "../../bindings";
import { ipc } from "../../ipc";
import { meetingMsNow } from "./clock";
import { parseNoteLine } from "./logic";

const TAGS: Array<{ kind: Exclude<NoteKind, "note">; icon: IconName; key: "decision" | "action" | "question"; digit: string }> = [
  { kind: "decision", icon: "check_circle", key: "decision", digit: "1" },
  { kind: "action", icon: "task_alt", key: "action", digit: "2" },
  { kind: "question", icon: "help", key: "question", digit: "3" },
];

const MARK_KEYS = { mac: "\u2318M", win: "Ctrl+M" } as const;

/** Exported so a discard can refresh the lines it removed. */
export const noteLinesKey = (meeting: string) => ["noteLines", meeting] as const;

function useNoteLines(meeting: string) {
  const { t } = useTranslation();
  const { show } = useToast();
  const qc = useQueryClient();
  const query = useQuery({
    queryKey: noteLinesKey(meeting),
    queryFn: async () => {
      const r = await ipc.commands.noteLines(meeting);
      if (r.status === "error") throw new Error(r.error);
      return r.data;
    },
    staleTime: Infinity,
  });
  const failed = (message: string) => show({ tone: "warning", title: t("system.commandFailed", { message }) });
  const set = (f: (ls: NoteLine[]) => NoteLine[]) => qc.setQueryData<NoteLine[]>(noteLinesKey(meeting), (ls) => f(ls ?? []));
  const add = async (text: string, kind: NoteKind, tMs: number | null) => {
    const r = await ipc.commands.addNoteLine(meeting, text, tMs, kind);
    if (r.status === "error") return failed(r.error);
    set((ls) => [...ls, r.data]);
  };
  return {
    lines: query.data ?? [],
    add,
    async update(gid: string, text: string) {
      const r = await ipc.commands.updateNoteLine(meeting, gid, text);
      if (r.status === "error") return failed(r.error);
      set((ls) => ls.map((l) => (l.gid === gid ? { ...l, text } : l)));
    },
    async remove(line: NoteLine) {
      const r = await ipc.commands.deleteNoteLine(meeting, line.gid);
      if (r.status === "error") return failed(r.error);
      set((ls) => ls.filter((l) => l.gid !== line.gid));
      // One click deletes, so it can be taken back (the line returns at the end, same time and tag).
      show({
        title: t("live.notepad.deleted"),
        action: { label: t("common.undo"), altText: t("common.undo"), onAction: () => void add(line.text, line.kind as NoteKind, line.tMs) },
      });
    },
  };
}

function Line({ line, onUpdate, onRemove }: { line: NoteLine; onUpdate: (text: string) => void; onRemove: () => void }) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(line.text);
  const tag = TAGS.find((x) => x.kind === line.kind);
  const view = parseNoteLine(line.text);
  const commit = () => {
    const text = draft.trim();
    setEditing(false);
    if (text && text !== line.text) onUpdate(text);
    else setDraft(line.text);
  };
  const onKey = (e: KeyboardEvent) => {
    // Enter that confirms an IME composition (Telex/VNI) must not save.
    if (e.key === "Enter" && !e.nativeEvent.isComposing) {
      e.preventDefault();
      commit();
    } else if (e.key === "Escape") {
      e.stopPropagation();
      setDraft(line.text);
      setEditing(false);
    }
  };
  return (
    <li data-kind={line.kind} className="group flex items-start gap-2 rounded-ctl px-2 hover:bg-sunk focus-within:bg-sunk">
      {tag ? <Icon name={tag.icon} size={16} label={t(`notes.tags.${tag.key}`)} className="mt-[7px] text-accent" /> : view.bullet ? <span aria-hidden="true" className="mt-px w-4 text-center text-muted">•</span> : null}
      {editing ? (
        <input
          autoFocus
          aria-label={t("speakers.line.edit")}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={onKey}
          onBlur={commit}
          className="h-8 min-w-0 flex-1 rounded-seg border border-ctl bg-surface px-2 font-serif text-[16px]"
        />
      ) : (
        <p className="m-0 min-w-0 flex-1 font-serif text-[16px] leading-[1.65] break-words">
          {view.parts.map((p, i) => (
            <Fragment key={i}>{p.bold ? <strong>{p.text}</strong> : p.italic ? <em>{p.text}</em> : p.text}</Fragment>
          ))}
        </p>
      )}
      {line.tMs != null && <span className="pt-1.5 text-mono text-[11px] text-muted opacity-0 group-focus-within:opacity-100 group-hover:opacity-100">{formatClock(line.tMs)}</span>}
      {!editing && (
        <span className="flex pt-1 opacity-0 group-focus-within:opacity-100 group-hover:opacity-100">
          <button type="button" onClick={() => setEditing(true)} aria-label={t("speakers.line.edit")} className="grid size-6 place-items-center rounded-seg text-muted hover:bg-sunk hover:text-ink">
            <Icon name="edit" size={14} />
          </button>
          <button type="button" onClick={onRemove} aria-label={t("common.delete")} className="grid size-6 place-items-center rounded-seg text-muted hover:bg-sunk hover:text-rec">
            <Icon name="delete" size={14} />
          </button>
        </span>
      )}
    </li>
  );
}

export function Notepad({ meeting, className, inert }: { meeting: string; className?: string; inert?: boolean }) {
  const { t } = useTranslation();
  const notes = useNoteLines(meeting);
  const [text, setText] = useState("");
  // A tag picked with nothing typed applies to the next line.
  const [pending, setPending] = useState<NoteKind>("note");
  const inputRef = useRef<HTMLInputElement>(null);
  const platform = usePlatform();
  const { run } = useAppActions();
  const marks = useLive((s) => s.marks.length);
  // The note belongs to the moment its first character was typed, not to Enter.
  const anchor = useRef<number | null>(null);

  const submit = (kind: NoteKind) => {
    const value = text.trim();
    if (!value) return;
    setText("");
    setPending("note");
    void notes.add(value, kind, anchor.current ?? meetingMsNow());
    anchor.current = null;
  };
  const tag = (kind: Exclude<NoteKind, "note">) => {
    if (text.trim()) submit(kind);
    else setPending((p) => (p === kind ? "note" : kind));
    inputRef.current?.focus();
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit(pending);
      return;
    }
    // Alt+1/2/3 (by key code: Option+digit types symbols on a Mac).
    const hit = e.altKey && !e.ctrlKey && !e.metaKey ? TAGS.find((x) => e.code === `Digit${x.digit}`) : undefined;
    if (hit) {
      e.preventDefault();
      tag(hit.kind);
    }
  };

  return (
    <section aria-label={t("live.yourNotes")} inert={inert} className={cn("group/pad flex min-h-0 flex-col rounded-panel border border-line bg-surface2", className)}>
      <header className="flex flex-none items-baseline gap-2.5 px-4 pt-3">
        <h2 className="m-0 text-[13px] font-bold">{t("live.yourNotes")}</h2>
        <p className="m-0 min-w-0 truncate text-[12px] text-faint">{t("live.padHint")}</p>
      </header>
      <div className="min-h-0 flex-1 overflow-auto px-2 py-2.5">
        <ul aria-label={t("live.yourNotes")} className="m-0 list-none p-0 empty:hidden">
          {notes.lines.map((l) => (
            <Line key={l.gid} line={l} onUpdate={(x) => void notes.update(l.gid, x)} onRemove={() => void notes.remove(l)} />
          ))}
        </ul>
        <input
          ref={inputRef}
          value={text}
          onChange={(e) => {
            if (!e.target.value.trim()) anchor.current = null;
            else anchor.current ??= meetingMsNow();
            setText(e.target.value);
          }}
          onKeyDown={onKeyDown}
          aria-label={t("live.notepad.label")}
          placeholder={t("live.notepad.placeholder")}
          className="h-9 w-full rounded-ctl bg-transparent px-2 font-serif text-[16px] placeholder:text-faint"
        />
      </div>
      <div className="flex flex-none flex-wrap items-center gap-2.5 border-t border-line px-2.5 py-2">
        <Button onClick={() => void run("mark")}>
          <Icon name="star" size={17} className="text-warn" />
          {t("live.mark")}
          <span className="text-mono text-[11px] font-normal text-faint">{MARK_KEYS[platform]}</span>
        </Button>
        {marks > 0 && <span className="text-[12px] text-muted">{t("live.markedCount", { count: marks })}</span>}
        {/* The tags (Alt+1/2/3) come up while the note field is in use. */}
        <div role="group" aria-label={t("live.notepad.tags")} className="ml-auto hidden gap-1.5 focus-within:flex group-has-[input:focus]/pad:flex">
          {TAGS.map((x) => (
            <Button key={x.kind} size="sm" icon={x.icon} aria-pressed={pending === x.kind} data-tag={x.kind} onMouseDown={(e) => e.preventDefault()} onClick={() => tag(x.kind)} className={cn(pending === x.kind && "border-accent bg-accent-soft text-accent")} title={platform === "mac" ? `⌥${x.digit}` : `Alt+${x.digit}`}>
              {t(`notes.tags.${x.key}`)}
            </Button>
          ))}
        </div>
      </div>
    </section>
  );
}
