// SPDX-License-Identifier: Apache-2.0
// One speaker paragraph of the transcript: who (avatar + name, never color
// alone) and when, then their lines. Each line plays on click, edits in place
// (double-click or the line menu) and can be moved to another speaker.
import { formatClock } from "@ghi/i18n";
import { memo, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar, Button, Icon, Menu, SpeakerChip, cn } from "@ghi/ui";
import type { MarkView, SegmentView } from "../../bindings";
import { usePlayer } from "../../state/player";
import { LineText, type LineRange } from "./line-text";
import type { GroupData } from "./logic";

export type SpeakerLabel = { name: string; initial?: string; colorSlot: number; isMe: boolean };

type GroupProps = {
  group: GroupData;
  /** Inside a stack: its header carries the hint, so each line only gets the short label. */
  stacked?: boolean;
  speaker: SpeakerLabel | null;
  /** Everyone who can take a line (for "change speaker"), with labels. */
  others: { gid: string; label: SpeakerLabel; numbered: boolean }[];
  active: number;
  ranges: ReadonlyMap<number, readonly LineRange[]>;
  marks: ReadonlyMap<number, readonly MarkView[]>;
  editing: string | null;
  picking: string | null;
  onEdit: (gid: string | null) => void;
  onPick: (gid: string | null) => void;
  onSave: (gid: string, text: string) => Promise<boolean>;
  onSetSpeaker: (gid: string, speaker: string, name: string) => Promise<boolean>;
};

const NONE: readonly LineRange[] = [];

function Editor({ seg, onSave, onCancel }: { seg: SegmentView; onSave: (text: string) => void; onCancel: () => void }) {
  const { t } = useTranslation();
  const ref = useRef<HTMLTextAreaElement>(null);
  const [text, setText] = useState(seg.text);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
  }, []);
  // Grows with its text (no scrollbar for a paragraph).
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [text]);
  const save = () => (text.trim() && text !== seg.text ? onSave(text.trim()) : onCancel());
  return (
    <div className="flex flex-col gap-1.5">
      <textarea
        ref={ref}
        value={text}
        rows={1}
        aria-label={t("speakers.line.edit")}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing) return; // Enter / Escape belong to the input method
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            onCancel();
          } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            save();
          }
        }}
        className="w-full resize-none rounded-ctl border border-accent bg-surface p-1.5 font-serif text-transcript text-ink outline-none"
      />
      <div className="flex gap-1.5">
        <Button variant="primary" size="sm" onClick={save}>
          {t("common.save")}
        </Button>
        <Button variant="ghost" size="sm" onClick={onCancel}>
          {t("common.cancel")}
        </Button>
      </div>
    </div>
  );
}

/** "Talking over each other": the line is less certain (`hint`: also on hover and for screen readers; a stack's header says it once). */
export function OverlapTag({ hint = true }: { hint?: boolean }) {
  const { t } = useTranslation();
  return (
    <span data-testid="overlap-tag" title={hint ? t("transcript.overlapHint") : undefined} className="inline-flex items-center gap-0.5 text-[11px] text-muted">
      <Icon name="forum" size={13} />
      {t("transcript.overlap")}
      {hint && <span className="sr-only">{`. ${t("transcript.overlapHint")}`}</span>}
    </span>
  );
}

function MarkTag({ mark }: { mark: MarkView }) {
  const { t } = useTranslation();
  const time = formatClock(mark.tMs ?? 0);
  const text = mark.tag === "decision" || mark.tag === "action" || mark.tag === "question" ? t(`notes.tags.${mark.tag}`) : t("live.markedToast", { time });
  return (
    <span data-testid="mark" className="inline-flex items-center gap-0.5 text-[11px] text-muted">
      <Icon name="star" size={14} className="text-warn" />
      {text}
    </span>
  );
}

export const GroupRow = memo(function GroupRow({ group, stacked, speaker, others, active, ranges, marks, editing, picking, onEdit, onPick, onSave, onSetSpeaker }: GroupProps) {
  const { t } = useTranslation();
  const first = group.segs[0]!;
  const time = formatClock(first.t0Ms ?? 0);
  const color = speaker && speaker.colorSlot > 0 ? `var(--s${speaker.colorSlot})` : undefined;
  const inGroup = active >= group.first && active < group.first + group.segs.length;

  return (
    <div data-testid="transcript-group" className={cn("grid grid-cols-[44px_26px_minmax(0,1fr)] gap-2.5 px-2 pt-2 pb-1")}>
      <button type="button" onClick={() => usePlayer.getState().seek(first.t0Ms ?? 0, true)} aria-label={t("detail.playFrom", { time })} className="h-6 self-start rounded-seg text-left text-mono text-[11px] text-muted hover:text-ink">
        {time}
      </button>
      {speaker ? <Avatar kind={speaker.isMe ? "me" : "person"} name={speaker.name} initial={speaker.initial} colorSlot={speaker.colorSlot} size="md" /> : <Avatar kind="unknown" size="md" />}
      <div className="min-w-0">
        <b className="block min-h-[22px] text-[13px]" style={color ? { color } : undefined}>
          {speaker ? speaker.name : t("notes.unassigned")}
        </b>
        {group.segs.map((seg, k) => {
          const index = group.first + k;
          const isActive = inGroup && index === active;
          const lineMarks = marks.get(index);
          return (
            <div
              key={seg.gid}
              data-seg={index}
              data-playing={isActive ? "true" : undefined}
              aria-current={isActive ? "true" : undefined}
              data-overlap={seg.overlap ? "true" : undefined}
              // An overlapped line is less certain: its text is muted like a low-confidence one.
              className={cn("group/line relative -mx-1.5 rounded-ctl px-1.5 py-0.5", isActive && "bg-accent-soft", seg.overlap && "[&_p]:text-muted")}
              onDoubleClick={() => editing !== seg.gid && onEdit(seg.gid)}
            >
              {editing === seg.gid ? (
                <Editor seg={seg} onSave={(text) => void onSave(seg.gid, text).then((ok) => ok && onEdit(null))} onCancel={() => onEdit(null)} />
              ) : (
                <>
                  <LineText seg={seg} active={isActive} ranges={ranges.get(index) ?? NONE} />
                  {(seg.edited || seg.overlap || lineMarks) && (
                    <div className="mt-0.5 flex flex-wrap items-center gap-2">
                      {seg.overlap && <OverlapTag hint={!stacked} />}
                      {seg.edited && (
                        <span className="inline-flex items-center gap-0.5 text-[11px] text-muted">
                          <Icon name="edit" size={13} />
                          {t("speakers.line.edited")}
                        </span>
                      )}
                      {lineMarks?.map((m, i) => <MarkTag key={i} mark={m} />)}
                    </div>
                  )}
                </>
              )}
              {editing !== seg.gid && (
                <div className="absolute top-0.5 right-0.5 rounded-ctl bg-surface opacity-0 shadow-float transition-opacity duration-(--motion-fast) group-focus-within/line:opacity-100 group-hover/line:opacity-100 has-[[data-state=open]]:opacity-100">
                  <Menu
                    label={t("transcript.lineMenu")}
                    trigger={
                      <button type="button" aria-label={t("transcript.lineMenu")} className="grid size-7 place-items-center rounded-seg text-muted hover:bg-sunk hover:text-ink">
                        <Icon name="more_horiz" size={16} />
                      </button>
                    }
                    items={[
                      { label: t("detail.playFrom", { time: formatClock(seg.t0Ms ?? 0) }), icon: "play_arrow", onSelect: () => usePlayer.getState().seek(seg.t0Ms ?? 0, true) },
                      { label: t("speakers.line.edit"), icon: "edit", onSelect: () => onEdit(seg.gid) },
                      { label: t("speakers.line.changeSpeaker"), icon: "person", onSelect: () => onPick(seg.gid) },
                    ]}
                  />
                </div>
              )}
              {picking === seg.gid && (
                <div
                  role="group"
                  aria-label={t("speakers.line.pickTarget")}
                  onKeyDown={(e) => e.key === "Escape" && (e.stopPropagation(), onPick(null))}
                  className="mt-1 flex flex-wrap items-center gap-1 rounded-ctl border border-line bg-surface p-1"
                >
                  <b className="px-1 text-[12px] font-semibold">{t("speakers.line.pickTarget")}</b>
                  {others
                    .filter((o) => o.gid !== seg.speakerGid)
                    .map((o) => (
                      <SpeakerChip
                        key={o.gid}
                        state={o.numbered ? "numbered" : "named"}
                        name={o.label.name}
                        colorSlot={o.label.colorSlot}
                        isMe={o.label.isMe}
                        onClick={() => void onSetSpeaker(seg.gid, o.gid, o.label.name).then((ok) => ok && onPick(null))}
                        className="border-transparent"
                      />
                    ))}
                  <Button variant="ghost" size="sm" onClick={() => onPick(null)}>
                    {t("common.cancel")}
                  </Button>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
});
