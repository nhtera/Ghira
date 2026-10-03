// SPDX-License-Identifier: Apache-2.0
// The live transcript: "Speaker N" lines, virtualized (a long meeting is
// thousands of lines), following the newest line until the user scrolls up.
// Words are text nodes only (RT-6). The list itself is not a live region: the
// screen reads out new speaker turns through `TurnAnnouncer` instead.
import { formatClock } from "@ghi/i18n";
import { Avatar, Icon, cn } from "@ghi/ui";
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { SpeakerInfo } from "../../bindings";
import { speakerInitial, type Line, type RecordModel } from "./model";

/** Closer than this (px) to the end counts as "at the latest line". */
const NEAR_END = 96;
/** A scroll within this long (ms) of a touch, wheel or key is the user's. */
const USER_MS = 1500;

type Props = {
  lines: Line[];
  partial: string;
  speakers: Record<number, SpeakerInfo>;
  className?: string;
};

const Row = memo(function Row({
  line,
  speaker,
  marked,
}: {
  line: Line;
  speaker: SpeakerInfo | undefined;
  marked: boolean;
}) {
  const { t } = useTranslation();
  const known = speaker && !speaker.provisional;
  const label = known ? speaker.label : t("mobile.record.identifying");
  const color =
    known && speaker.colorSlot > 0 ? `var(--s${speaker.colorSlot})` : undefined;
  return (
    <div data-testid="line" className="flex gap-3 px-4 py-2.5">
      {known ? (
        <Avatar
          kind={speaker.isMe ? "me" : "person"}
          name={speaker.label}
          initial={speakerInitial(speaker.label)}
          colorSlot={speaker.colorSlot}
          size="md"
        />
      ) : (
        <Avatar kind="unknown" size="md" />
      )}
      <div className="min-w-0 flex-1">
        <div className="text-ios-footnote flex flex-wrap items-center gap-x-2">
          <b
            style={color ? { color } : undefined}
            className={cn(!known && "font-semibold text-muted italic")}
          >
            {label}
          </b>
          <span className="font-mono text-muted tabular-nums">
            {formatClock(line.t0Ms)}
          </span>
          {marked && (
            <Icon
              name="star"
              size={16}
              label={t("mobile.record.mark")}
              className="size-4 text-warn"
            />
          )}
        </div>
        <p className="text-ios-body m-0 mt-0.5 font-serif break-words text-ink">
          {line.text}
        </p>
      </div>
    </div>
  );
});

export const LiveTranscript = memo(function LiveTranscript({ lines, partial, speakers, className }: Props) {
  const { t } = useTranslation();
  const scroller = useRef<HTMLDivElement>(null);
  const [following, setFollowing] = useState(true);
  /** When the user last touched, wheeled or keyed the list. */
  const touched = useRef(0);
  const count = lines.length + (partial ? 1 : 0);

  // eslint-disable-next-line react-hooks/incompatible-library -- the list only reads the virtualizer during this render
  const virtual = useVirtualizer({
    count,
    getScrollElement: () => scroller.current,
    estimateSize: () => 84,
    overscan: 8,
    getItemKey: (i) => (i < lines.length ? lines[i].key : "partial"),
  });
  const total = virtual.getTotalSize();

  // Follow the newest line while the user is at the end.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && following) el.scrollTop = el.scrollHeight;
  }, [count, partial, total, following]);

  // Only the user lets go of the end: lines growing taller after they are
  // measured push the end away without any touch.
  const touch = () => {
    touched.current = Date.now();
  };
  const onScroll = () => {
    const el = scroller.current;
    if (!el) return;
    if (el.scrollHeight - el.scrollTop - el.clientHeight < NEAR_END)
      setFollowing(true);
    else if (Date.now() - touched.current < USER_MS) setFollowing(false);
  };

  return (
    <div className={cn("relative min-h-0", className)}>
      <div
        ref={scroller}
        role="log"
        aria-live="off"
        aria-label={t("mobile.record.transcript")}
        tabIndex={0}
        onScroll={onScroll}
        onTouchMove={touch}
        onWheel={touch}
        onKeyDown={touch}
        className="h-full overflow-y-auto overscroll-contain rounded-(--ios-radius-group) bg-surface"
      >
        {count === 0 ? (
          <p className="text-ios-subhead m-0 p-4 text-center text-muted">
            {t("mobile.record.waiting")}
          </p>
        ) : (
          <div style={{ height: total }} className="relative w-full">
            {virtual.getVirtualItems().map((item) => (
              <div
                key={item.key}
                ref={virtual.measureElement}
                data-index={item.index}
                style={{ transform: `translateY(${item.start}px)` }}
                className="absolute top-0 left-0 w-full"
              >
                {item.index < lines.length ? (
                  <Row
                    line={lines[item.index]}
                    speaker={
                      lines[item.index].speaker === null
                        ? undefined
                        : speakers[lines[item.index].speaker!]
                    }
                    marked={lines[item.index].marked}
                  />
                ) : (
                  <p
                    data-testid="partial"
                    className="text-ios-body m-0 px-4 py-2.5 font-serif text-muted italic"
                  >
                    {partial}
                  </p>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
      {!following && (
        <button
          type="button"
          onClick={() => setFollowing(true)}
          className="text-ios-footnote absolute right-3 bottom-3 inline-flex min-h-ios-target items-center gap-1 rounded-full bg-ink px-4 font-semibold text-bg shadow-float"
        >
          <Icon name="expand_more" size={20} className="size-5" />
          {t("mobile.record.jumpLatest")}
        </button>
      )}
    </div>
  );
});

/** Reads out a new speaker turn ("Speaker 2 speaking"); not every line. */
export function TurnAnnouncer({
  announce,
}: {
  announce: RecordModel["announce"];
}) {
  const { t } = useTranslation();
  return (
    <div role="status" aria-live="polite" className="sr-only">
      {announce ? t("mobile.record.speaking", { name: announce.name }) : ""}
    </div>
  );
}
