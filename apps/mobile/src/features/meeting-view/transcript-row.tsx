// SPDX-License-Identifier: Apache-2.0
// One transcript line for the phone, as in the design: the avatar on the left,
// the coloured name and the time on one line, the words in serif below. A
// single button (VoiceOver and keyboard reach it) that selects the line; sizes
// in rem, so time, name and text grow with Dynamic Type. The speaker is color +
// initial, never color alone.
import { formatClock } from "@ghi/i18n";
import {
  Avatar,
  cn,
  Icon,
  type TranscriptSpeaker,
  type TranscriptWord,
} from "@ghi/ui";
import { Fragment } from "react";
import type { MarkView } from "../../bindings";
import { useTranslation } from "react-i18next";

export type TranscriptRowProps = {
  startMs: number;
  speaker: TranscriptSpeaker | null;
  words: TranscriptWord[];
  edited: boolean;
  overlap: boolean;
  /** Moments marked while recording that fall on this line. */
  marks?: readonly MarkView[];
  playing: boolean;
  activeWordIndex?: number;
  selected: boolean;
  onSelect: () => void;
};

export function TranscriptRow({
  startMs,
  speaker,
  words,
  edited,
  overlap,
  marks = [],
  playing,
  activeWordIndex,
  selected,
  onSelect,
}: TranscriptRowProps) {
  const { t } = useTranslation();
  const color =
    speaker && speaker.colorSlot > 0
      ? `var(--s${speaker.colorSlot})`
      : undefined;
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-expanded={selected}
      data-playing={playing ? "true" : undefined}
      data-selected={selected ? "true" : undefined}
      aria-current={playing ? "true" : undefined}
      className={cn(
        "grid min-h-ios-target w-full grid-cols-[1.5rem_minmax(0,1fr)] gap-x-2.5 rounded-ctl border-[1.5px] px-2 py-2.5 text-start",
        playing
          ? "border-transparent bg-accent-soft"
          : selected
            ? "border-accent bg-surface2"
            : "border-transparent",
      )}
    >
      <span className="flex flex-col items-start">
        {speaker ? (
          <Avatar
            kind={speaker.isMe ? "me" : "person"}
            name={speaker.label}
            initial={speaker.initial}
            colorSlot={speaker.colorSlot}
            size="md"
          />
        ) : (
          <Avatar kind="unknown" size="md" />
        )}
      </span>
      <span className="min-w-0">
        <span className="flex flex-wrap items-baseline gap-x-1.5">
          <b
            className={cn(
              "text-ios-footnote font-bold",
              !speaker && "text-muted italic",
            )}
            style={color && !playing ? { color } : undefined}
          >
            {speaker ? speaker.label : t("speakers.identifying")}
          </b>
          <span className="text-ios-caption1 font-mono text-muted tabular-nums">
            {formatClock(startMs, { pad: true })}
          </span>
          {overlap && (
            <span className="text-ios-caption1 inline-flex items-center gap-0.5 text-muted">
              <Icon name="forum" size={13} />
              {t("transcript.overlap")}
            </span>
          )}
          {marks.map((m, i) => (
            <span
              key={i}
              data-testid="mark"
              className="text-ios-caption1 inline-flex items-center gap-0.5 text-muted"
            >
              <Icon name="star" size={13} className="text-warn" />
              {t(
                `notes.tags.${m.tag === "decision" || m.tag === "action" || m.tag === "question" ? m.tag : "star"}`,
              )}
            </span>
          ))}
          {edited && (
            <span className="text-ios-caption1 inline-flex items-center gap-0.5 text-muted">
              <Icon name="edit" size={13} />
              {t("speakers.line.edited")}
            </span>
          )}
        </span>
        <span
          className={cn(
            "text-ios-body mt-0.5 block font-serif",
            overlap ? "text-muted" : "text-ink",
          )}
        >
          {words.map((w, i) => (
            <Fragment key={i}>
              {i > 0 && " "}
              <span
                data-low={w.lowConfidence ? "true" : undefined}
                data-active={
                  playing && i === activeWordIndex ? "true" : undefined
                }
                className={cn(
                  w.lowConfidence &&
                    "underline decoration-warn decoration-dotted decoration-2 underline-offset-4",
                  playing &&
                    i === activeWordIndex &&
                    "font-semibold text-accent underline decoration-accent decoration-2 underline-offset-4",
                )}
              >
                {w.text}
                {w.lowConfidence && (
                  <span className="sr-only">{` (${t("detail.lowConfidence")})`}</span>
                )}
              </span>
            </Fragment>
          ))}
        </span>
      </span>
    </button>
  );
}
