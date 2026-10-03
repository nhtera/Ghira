// SPDX-License-Identifier: Apache-2.0
// One transcript line (brief §7): time · avatar · name · text. Words are text
// nodes only (RT-6). States: partial, final, provisional speaker, low
// confidence words, marked, playing (karaoke), selected, edited.
import { formatClock } from "@ghi/i18n";
import { Fragment } from "react";
import { useTranslation } from "react-i18next";
import { Icon } from "../../icons/icon";
import { cn } from "../../utils/cn";
import { Avatar } from "../avatar";

export type TranscriptWord = { text: string; lowConfidence?: boolean };

/** Splits plain text on spaces; `low` marks the words it contains ("CoreML.") as low confidence. */
export function wordsFromText(text: string, low = ""): TranscriptWord[] {
  const flagged = new Set(low.split(/\s+/).filter(Boolean));
  return text.split(/\s+/).filter(Boolean).map((w) => ({ text: w, lowConfidence: flagged.has(w) }));
}

export type TranscriptSpeaker = {
  label: string;
  colorSlot: number;
  isMe?: boolean;
  /** Avatar text when not the label's first letter (an unnamed speaker's number). */
  initial?: string;
};

export type TranscriptLineProps = {
  startMs: number;
  /** `null` while the speaker is still being identified (provisional). */
  speaker: TranscriptSpeaker | null;
  words: TranscriptWord[];
  /** Live line still being recognized: muted, the newest words pulse. */
  partial?: boolean;
  marked?: boolean;
  edited?: boolean;
  /** Another speaker talked over this line: marker + hint, text muted like a low-confidence one. */
  overlap?: boolean;
  /** With `overlap`: the hint too (tooltip, screen readers). Off inside a stack, whose header says it once. */
  overlapHint?: boolean;
  /** This line is being played. */
  playing?: boolean;
  /** Karaoke: only this word is highlighted (index into `words`). */
  activeWordIndex?: number;
  selected?: boolean;
  /** Click on the time: play from here. */
  onPlay?: () => void;
  onEdit?: () => void;
  onChangeSpeaker?: () => void;
  className?: string;
};

const TAIL = 2;

export function TranscriptLine({ startMs, speaker, words, partial, marked, edited, overlap, overlapHint = true, playing, activeWordIndex, selected, onPlay, onEdit, onChangeSpeaker, className }: TranscriptLineProps) {
  const { t } = useTranslation();
  const time = formatClock(startMs);
  const color = speaker && speaker.colorSlot > 0 ? `var(--s${speaker.colorSlot})` : undefined;

  return (
    <div
      data-state={partial ? "partial" : "final"}
      data-playing={playing ? "true" : undefined}
      data-overlap={overlap ? "true" : undefined}
      data-selected={selected ? "true" : undefined}
      aria-current={playing ? "true" : undefined}
      className={cn(
        "group relative grid grid-cols-[44px_26px_minmax(0,1fr)] gap-2.5 rounded-ctl border-[1.5px] p-2",
        playing ? "border-transparent bg-accent-soft" : selected ? "border-accent bg-surface2" : "border-transparent",
        className,
      )}
    >
      {onPlay ? (
        <button type="button" onClick={onPlay} aria-label={t("detail.playFrom", { time })} className="h-6 self-start rounded-seg text-left text-mono text-[11px] text-muted hover:text-ink">
          {time}
        </button>
      ) : (
        <span className="pt-[5px] text-mono text-[11px] text-muted">{time}</span>
      )}
      {speaker ? <Avatar kind={speaker.isMe ? "me" : "person"} name={speaker.label} initial={speaker.initial} colorSlot={speaker.colorSlot} size="md" /> : <Avatar kind="unknown" size="md" />}
      <div className="min-w-0">
        <div className="flex min-h-[22px] items-center gap-1.5">
          <b className={cn("text-[13px]", !speaker && "font-semibold text-muted italic", playing && "text-ink")} style={color && !playing ? { color } : undefined}>
            {speaker ? speaker.label : t("speakers.identifying")}
          </b>
          {marked && <Icon name="star" size={16} label={t("live.markedToast", { time })} className="text-warn" />}
          {overlap && (
            <span data-testid="overlap-tag" title={overlapHint ? t("transcript.overlapHint") : undefined} className="inline-flex items-center gap-0.5 text-[11px] text-muted">
              <Icon name="forum" size={13} />
              {t("transcript.overlap")}
              {overlapHint && <span className="sr-only">{`. ${t("transcript.overlapHint")}`}</span>}
            </span>
          )}
          {edited && (
            <span className="inline-flex items-center gap-0.5 text-[11px] text-muted">
              <Icon name="edit" size={13} />
              {t("speakers.line.edited")}
            </span>
          )}
        </div>
        <p aria-live="off" className={cn("m-0 mt-0.5 font-serif text-transcript", partial || overlap ? "text-muted" : "text-ink")}>
          {words.map((w, i) => (
            <Fragment key={i}>
              {i > 0 && " "}
              <span
                data-low={w.lowConfidence ? "true" : undefined}
                data-active={playing && i === activeWordIndex ? "true" : undefined}
                title={w.lowConfidence ? t("detail.lowConfidence") : undefined}
                className={cn(
                  "rounded-[3px]",
                  w.lowConfidence && "underline decoration-warn decoration-dotted decoration-2 underline-offset-4",
                  playing && i === activeWordIndex && "font-semibold text-accent underline decoration-accent decoration-2 underline-offset-4",
                  partial && i >= words.length - TAIL && "border-b-2 border-accent animate-pulse motion-reduce:animate-none",
                )}
              >
                {w.text}
                {w.lowConfidence && <span className="sr-only">{` (${t("detail.lowConfidence")})`}</span>}
              </span>
            </Fragment>
          ))}
        </p>
      </div>
      {(onEdit || onChangeSpeaker) && (
        <div className="absolute top-1.5 right-1.5 flex gap-0.5 rounded-ctl bg-surface p-0.5 opacity-0 shadow-float transition-opacity duration-(--motion-fast) group-focus-within:opacity-100 group-hover:opacity-100">
          {onEdit && (
            <button type="button" onClick={onEdit} aria-label={t("speakers.line.edit")} className="grid size-7 place-items-center rounded-seg text-muted hover:bg-sunk hover:text-ink">
              <Icon name="edit" size={15} />
            </button>
          )}
          {onChangeSpeaker && (
            <button type="button" onClick={onChangeSpeaker} aria-label={t("speakers.line.changeSpeaker")} className="grid size-7 place-items-center rounded-seg text-muted hover:bg-sunk hover:text-ink">
              <Icon name="person" size={15} />
            </button>
          )}
        </div>
      )}
    </div>
  );
}
