// SPDX-License-Identifier: Apache-2.0
// Design 7c "Speaker side panel": everything you can do with one speaker of a
// STORED meeting, for the whole meeting: rename, Me, merge, split, not a
// person. Opened from a speaker's name on a transcript line (the live strip
// keeps its popover). A modal panel: focus stays inside, Escape closes, focus
// returns to where it came from. Merge and "not a person" ask first.
import { formatClock } from "@ghi/i18n";
import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Avatar, Button, Icon, InlineConfirm, SpeakerChip } from "@ghi/ui";
import type { MeetingSpeaker, SegmentView } from "../../bindings";
import { ipc } from "../../ipc";
import { speakerDisplay } from "../meeting/speaker-display";
import { errorText } from "../people/error-text";
import { isPanelError, linesFrom, mergeTargets, ownLines, splitAllowed, splitCount, type SplitChoice } from "./logic";
import { usePanelActions, type Outcome } from "./use-panel-actions";

const FOCUSABLE = 'button:not([disabled]), input:not([disabled]), [href], [tabindex]:not([tabindex="-1"])';

type Ask = { kind: "merge"; into: string } | { kind: "notPerson" } | null;

export type SpeakerPanelProps = {
  meeting: string;
  /** `call` or `room`: in a call only the mic speaker can be Me, and the core decides. */
  mode: string;
  speaker: MeetingSpeaker;
  speakers: readonly MeetingSpeaker[];
  /** The whole transcript. */
  segments: readonly SegmentView[];
  /** The line the panel was opened from, for "from this line on". */
  fromSegment: string | null;
  onClose: () => void;
  /** Called with the speaker's gid on a merge: they no longer exist. */
  onMerged?: () => void;
};

export function SpeakerPanel({ meeting, mode, speaker, speakers, segments, fromSegment, onClose, onMerged }: SpeakerPanelProps) {
  const { t } = useTranslation();
  const actions = usePanelActions(meeting);
  const root = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const d = speakerDisplay(speaker, t);
  const own = useMemo(() => ownLines(segments, speaker.gid), [segments, speaker.gid]);
  const targets = useMemo(() => mergeTargets(speakers, speaker.gid), [speakers, speaker.gid]);

  // What is typed; null: the stored name (so a rename or a reload shows through).
  const [draft, setDraft] = useState<string | null>(null);
  const name = draft ?? speaker.name ?? "";
  const [error, setError] = useState<string | null>(null);
  const [ask, setAsk] = useState<Ask>(null);
  const [splitting, setSplitting] = useState(false);
  const [splitMode, setSplitMode] = useState<"from" | "lines">("from");
  const [picked, setPicked] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  // Under the 40 px title bar and above the audio bar, as in the design: inside the content area.
  const [bottom] = useState(() => {
    const bar = document.querySelector("[data-testid=audio-bar]");
    return bar ? Math.max(0, window.innerHeight - bar.getBoundingClientRect().top) : 0;
  });
  const [sample, setSample] = useState<string | null>(null);

  // Focus goes in on open and back to the opener on close (the opener may have
  // been redrawn by the reload: then the same speaker's name on the page).
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    root.current?.querySelector<HTMLElement>("input")?.focus();
    return () => {
      // WebKit does not focus a button on click: then the opener is the page (or a scroll area), and the speaker's name is the way back.
      const target = opener?.isConnected && opener.matches("button, input, a[href]") ? opener : document.querySelector<HTMLElement>(`[data-speaker-open="${speaker.gid}"]`);
      target?.focus();
    };
    // Once per opening.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // "From this line on" is offered while the line is still the speaker's (after a split it moved away).
  const fromLine = fromSegment && linesFrom(own, fromSegment).length > 0 ? fromSegment : null;
  const choice: SplitChoice = splitMode === "from" && fromLine ? { mode: "from", from: fromLine } : { mode: "lines", picked };
  const splitKind = choice.mode;
  const moving = splitCount(own, choice);

  const attempt = async (call: Promise<Outcome>, after?: () => void) => {
    setBusy(true);
    setError(null);
    const r = await call;
    setBusy(false);
    if (r.ok) after?.();
    else setError(r.error);
    return r.ok;
  };
  // The panel's own sentences first; the other voice codes (notMe, …) are People's.
  const sentence = (code: string) => (isPanelError(code) ? t(`speakerPanel.errors.${code}`) : errorText(t, code));

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      onClose();
    } else if (e.key === "Tab" && root.current) {
      // Focus on the panel itself (a click on its padding): Tab enters at the first control, Shift+Tab at the last.
      if (document.activeElement === root.current) {
        e.preventDefault();
        const all = root.current.querySelectorAll<HTMLElement>(FOCUSABLE);
        (e.shiftKey ? all[all.length - 1] : all[0])?.focus();
        return;
      }
      const items = [...root.current.querySelectorAll<HTMLElement>(FOCUSABLE)];
      const first = items[0];
      const last = items.at(-1);
      if (!first || !last) return;
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  };

  const playSample = async () => {
    if (speaker.sampleT0Ms == null || speaker.sampleT1Ms == null) return;
    const r = await ipc.commands.issueAudioSample(meeting, speaker.sampleT0Ms, speaker.sampleT1Ms, null);
    // No audio (the mock core, a deleted bundle): the rest of the panel still works.
    if (r.status === "ok") setSample(ipc.audioUrl(r.data));
  };

  const mergeInto = ask?.kind === "merge" ? targets.find((s) => s.gid === ask.into) : undefined;
  const nameOf = (s: MeetingSpeaker) => speakerDisplay(s, t).name;
  const trimmed = name.trim();

  return createPortal(
    <div
      ref={root}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      data-testid="speaker-panel"
      tabIndex={-1}
      onKeyDown={onKeyDown}
      // Focus never leaves a modal panel: Escape and the Tab trap keep working.
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) e.currentTarget.focus();
      }}
      style={{ bottom }}
      className="fixed top-10 right-0 z-30 flex outline-none w-[380px] max-w-full flex-col gap-4 overflow-auto border-l border-line2 bg-surface p-5 text-ink shadow-float"
    >
      <div className="flex items-start gap-3">
        <Avatar kind={d.isMe ? "me" : "person"} name={d.name} initial={d.initial} colorSlot={d.colorSlot} size="xl" />
        <div className="min-w-0 flex-1">
          <span className="block text-[12px] text-muted">{t("speakerPanel.caption", { number: speaker.number })}</span>
          <h2 id={titleId} className="m-0 truncate text-[17px] font-semibold">
            {d.name}
          </h2>
          <span className="block text-[12.5px] text-muted">
            {t("speakers.linesInMeeting", { count: speaker.lines })}
            {speaker.notPerson && ` · ${t("speakers.notPerson.row")}`}
          </span>
        </div>
        <button type="button" aria-label={t("speakers.close")} onClick={onClose} className="grid size-7 place-items-center rounded-seg text-faint hover:bg-sunk">
          <Icon name="close" size={18} />
        </button>
      </div>

      {error && (
        <p role="alert" data-testid="speaker-panel-error" className="m-0 flex items-start gap-2 rounded-ctl bg-rec-soft px-3 py-2 text-[13px] text-rec-ink">
          <Icon name="error" size={16} className="mt-0.5 shrink-0" />
          {sentence(error)}
        </p>
      )}

      {speaker.sampleT0Ms != null && (
        <div className="flex items-center gap-2">
          <Button icon="play_arrow" onClick={() => void playSample()}>
            {t("speakers.playSample")}
          </Button>
          {sample && <audio key={sample} src={sample} autoPlay aria-label={t("speakers.playing")} />}
        </div>
      )}

      <form
        className="flex flex-col gap-1.5"
        onSubmit={(e) => {
          e.preventDefault();
          if (trimmed !== (speaker.name ?? "")) void attempt(actions.rename(speaker.gid, trimmed), () => setDraft(null));
        }}
      >
        <div className="flex gap-2">
          <input
            value={name}
            onChange={(e) => setDraft(e.target.value)}
            aria-label={t("speakers.rename")}
            placeholder={t("speakers.typeName")}
            className="h-[38px] min-w-0 flex-1 rounded-seg border border-ctl bg-surface px-3 text-[14px] focus:border-accent"
          />
          <Button type="submit" variant="primary" size="lg" disabled={busy || trimmed === (speaker.name ?? "")}>
            {t("common.save")}
          </Button>
        </div>
        <p className="m-0 text-[12px] text-muted">{t("speakerPanel.nameHint", { number: speaker.number })}</p>
      </form>

      {mode !== "call" && !speaker.notPerson && (
        <Button
          icon={speaker.isMe ? "person_remove" : "person"}
          disabled={busy}
          onClick={() => void attempt(actions.setMe(speaker.gid, !speaker.isMe))}
          className="self-start"
        >
          {speaker.isMe ? t("speakers.notMe") : t("speakers.thisIsMe")}
        </Button>
      )}

      {targets.length > 0 && (
        <section aria-labelledby={`${titleId}-merge`} className="flex flex-col gap-2 border-t border-line pt-3">
          <h3 id={`${titleId}-merge`} className="m-0 text-[13px] font-semibold text-muted">
            {t("speakers.mergeInto")}
          </h3>
          {mergeInto ? (
            <InlineConfirm
              icon="call_merge"
              question={t("speakerPanel.mergeConfirm", { from: d.name, into: nameOf(mergeInto), count: speaker.lines })}
              confirmLabel={t("speakerPanel.mergeAction")}
              onCancel={() => setAsk(null)}
              onConfirm={() => {
                setAsk(null);
                void attempt(actions.merge(speaker.gid, mergeInto.gid, nameOf(mergeInto)), onMerged);
              }}
            />
          ) : (
            <fieldset disabled={busy} className="m-0 min-w-0 border-0 p-0">
            <ul className="m-0 flex list-none flex-wrap gap-1.5 p-0">
              {targets.map((s) => {
                const td = speakerDisplay(s, t);
                return (
                  <li key={s.gid}>
                    <SpeakerChip
                      state={td.named ? "named" : "numbered"}
                      name={td.name}
                      colorSlot={td.colorSlot}
                      isMe={td.isMe}
                      onClick={() => {
                        setError(null);
                        setAsk({ kind: "merge", into: s.gid });
                      }}
                    />
                  </li>
                );
              })}
            </ul>
            </fieldset>
          )}
        </section>
      )}

      <section className="flex flex-col gap-2 border-t border-line pt-3">
        <button
          type="button"
          aria-expanded={splitting}
          disabled={busy}
          onClick={() => setSplitting((v) => !v)}
          className="text-body flex h-8 w-full items-center gap-2 rounded-seg px-1 text-left hover:bg-sunk"
        >
          <Icon name="call_split" size={16} />
          {t("speakers.split.action")}
        </button>
        {splitting && (
          <div className="flex flex-col gap-2">
            <p className="text-small m-0 text-muted">{t("speakerPanel.splitBody")}</p>
            {fromLine && (
              <label className="text-small flex cursor-pointer items-start gap-2 px-1">
                <input type="radio" name={`${titleId}-split`} checked={splitKind === "from"} onChange={() => setSplitMode("from")} className="mt-0.5" />
                <span>{t("speakerPanel.fromLine", { count: linesFrom(own, fromLine).length, time: formatClock(own.find((s) => s.gid === fromLine)?.t0Ms ?? 0, { pad: true }) })}</span>
              </label>
            )}
            <label className="text-small flex cursor-pointer items-start gap-2 px-1">
              <input type="radio" name={`${titleId}-split`} checked={splitKind === "lines"} onChange={() => setSplitMode("lines")} className="mt-0.5" />
              <span>{t("speakerPanel.theseLines")}</span>
            </label>
            {splitKind === "lines" && (
              <ul aria-label={t("speakerPanel.theseLines")} className="m-0 flex max-h-56 list-none flex-col gap-0.5 overflow-auto p-0">
                {own.map((l) => (
                  <li key={l.gid}>
                    <label className="text-small flex cursor-pointer items-start gap-2 rounded-seg px-1.5 py-1 hover:bg-sunk">
                      <input
                        type="checkbox"
                        checked={picked.includes(l.gid)}
                        onChange={() => setPicked((p) => (p.includes(l.gid) ? p.filter((x) => x !== l.gid) : [...p, l.gid]))}
                        className="mt-0.5"
                      />
                      <span className="min-w-0 flex-1">
                        <span className="text-mono mr-1.5 text-[11px] text-muted">{formatClock(l.t0Ms ?? 0, { pad: true })}</span>
                        {l.text}
                      </span>
                    </label>
                  </li>
                ))}
              </ul>
            )}
            <Button
              variant="primary"
              disabled={busy || !splitAllowed(own, choice)}
              onClick={() =>
                void attempt(actions.split(speaker.gid, choice, moving), () => {
                  setSplitting(false);
                  setPicked([]);
                  setSplitMode("from");
                })
              }
              className="self-start"
            >
              {t("speakers.split.go", { count: moving })}
            </Button>
          </div>
        )}
      </section>

      <section className="flex flex-col gap-2 border-t border-line pt-3">
        {speaker.notPerson ? (
          <Button icon="person" disabled={busy} onClick={() => void attempt(actions.notPerson(speaker.gid, false, d.name))} className="self-start">
            {t("speakers.notPerson.restore")}
          </Button>
        ) : ask?.kind === "notPerson" ? (
          <InlineConfirm
            icon="block"
            question={t("speakerPanel.notPersonConfirm", { name: d.name })}
            confirmLabel={t("speakerPanel.notPersonAction")}
            onCancel={() => setAsk(null)}
            onConfirm={() => {
              setAsk(null);
              void attempt(actions.notPerson(speaker.gid, true, d.name));
            }}
          />
        ) : (
          <button
            type="button"
            disabled={busy}
            onClick={() => {
              setError(null);
              setAsk({ kind: "notPerson" });
            }}
            className="text-body flex h-8 w-full items-center gap-2 rounded-seg px-1 text-left text-rec hover:bg-sunk"
          >
            <Icon name="block" size={16} />
            {t("speakers.notPerson.action")}
          </button>
        )}
      </section>
    </div>,
    document.body,
  );
}
