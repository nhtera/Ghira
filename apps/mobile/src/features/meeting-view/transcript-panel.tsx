// SPDX-License-Identifier: Apache-2.0
// M4 Transcript tab: a virtualized list of lines (hours of speech), the line
// being played highlighted word by word; tap a line to play from there or edit it.
import { formatClock } from "@ghi/i18n";
import { Button } from "@ghi/ui";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type RefObject,
} from "react";
import { useTranslation } from "react-i18next";
import type { MarkView, MeetingSpeaker, SegmentView } from "../../bindings";
import { LOCKED_EVENT } from "../app-lock/events";
import { useWindowEvent } from "./use-window-event";
import { TranscriptRow } from "./transcript-row";
import {
  activeSegment,
  marksBySegment,
  activeWord,
  segmentNear,
  segmentWords,
  speakerOf,
  transcriptSpeaker,
} from "./notes-model";

export type TranscriptPanelProps = {
  segments: SegmentView[];
  /** Moments marked while recording (read-only). */
  marks?: MarkView[];
  speakers: MeetingSpeaker[];
  scroller: RefObject<HTMLElement | null>;
  /** Audio position while playing or paused mid-way. */
  timeMs: number;
  playing: boolean;
  /** Open with the line nearest this moment selected (a search result). */
  focusAt?: number;
  /** The meeting still has audio (a sensitive one has none). */
  canPlay?: boolean;
  onPlay: (ms: number) => void;
  onSave: (segment: string, text: string) => Promise<boolean>;
  /** The transcript is not there yet (waiting for models) or empty. */
  empty: string;
  /** A final pass is writing it elsewhere: lines can be read and played, not edited. */
  readOnly?: boolean;
};

const ESTIMATE = 96;

export function TranscriptPanel({
  segments,
  marks = [],
  speakers,
  scroller,
  timeMs,
  playing,
  focusAt,
  canPlay = true,
  onPlay,
  onSave,
  empty,
  readOnly = false,
}: TranscriptPanelProps) {
  const { t } = useTranslation();
  const listRef = useRef<HTMLDivElement>(null);
  const [margin, setMargin] = useState(0);
  const [selected, setSelected] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [saveFailed, setSaveFailed] = useState(false);
  // Locking hides the text: drop the selection and any half-typed edit.
  useWindowEvent(LOCKED_EVENT, () => {
    setSelected(null);
    setEditing(null);
    setDraft("");
  });
  const numbered = (number: number) => t("speakers.numbered", { number });

  useLayoutEffect(() => {
    if (listRef.current) setMargin(listRef.current.offsetTop);
  }, [segments.length]);

  const marksOf = useMemo(() => marksBySegment(marks), [marks]);
  // eslint-disable-next-line react-hooks/incompatible-library -- the virtualizer is only used in this component
  const virtual = useVirtualizer({
    count: segments.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => ESTIMATE,
    getItemKey: (i) => segments[i].gid,
    scrollMargin: margin,
    overscan: 6,
  });

  const active = playing || timeMs > 0 ? activeSegment(segments, timeMs) : -1;
  useEffect(() => {
    if (playing && active >= 0)
      virtual.scrollToIndex(active, { align: "center" });
    // Follow the playing line, not every render of the virtualizer.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, playing]);

  const focused = useRef(false);
  useEffect(() => {
    if (focused.current || focusAt === undefined || segments.length === 0)
      return;
    focused.current = true;
    const i = segmentNear(segments, focusAt);
    if (i < 0) return;
    setSelected(segments[i].gid);
    virtual.scrollToIndex(i, { align: "center" });
  }, [focusAt, segments, virtual]);

  if (segments.length === 0)
    return (
      <p className="text-ios-subhead m-0 px-6 py-10 text-center text-muted">
        {empty}
      </p>
    );

  const save = async (s: SegmentView) => {
    const text = draft.trim();
    if (text && text !== s.text && !(await onSave(s.gid, text)))
      return setSaveFailed(true);
    setSaveFailed(false);
    setEditing(null);
  };

  return (
    <div
      ref={listRef}
      style={{ height: virtual.getTotalSize(), position: "relative" }}
    >
      {virtual.getVirtualItems().map((v) => {
        const s = segments[v.index];
        const speaker = transcriptSpeaker(
          speakerOf(speakers, s.speakerGid),
          numbered,
          t("speakers.me"),
        );
        return (
          <div
            key={v.key}
            data-index={v.index}
            data-segment={s.gid}
            ref={virtual.measureElement}
            style={{
              position: "absolute",
              top: 0,
              left: 0,
              width: "100%",
              transform: `translateY(${v.start - margin}px)`,
            }}
            className="px-2 py-0.5"
          >
            {editing === s.gid ? (
              <div className="flex flex-col gap-2 rounded-ctl border-[1.5px] border-accent bg-surface p-2">
                <textarea
                  autoFocus
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  aria-label={t("mobile.detail.editField")}
                  rows={3}
                  className="text-transcript w-full resize-none rounded-seg border border-line bg-bg p-2 font-serif text-ink select-text"
                />
                {saveFailed && (
                  <p
                    role="alert"
                    className="text-ios-footnote m-0 text-rec-ink"
                  >
                    {t("mobile.detail.saveFailed")}
                  </p>
                )}
                <div className="flex justify-end gap-2">
                  <Button
                    className="min-h-ios-target px-4"
                    onClick={() => {
                      setSaveFailed(false);
                      setEditing(null);
                    }}
                  >
                    {t("mobile.common.cancel")}
                  </Button>
                  <Button
                    variant="primary"
                    className="min-h-ios-target px-4"
                    onClick={() => void save(s)}
                  >
                    {t("mobile.detail.save")}
                  </Button>
                </div>
              </div>
            ) : (
              <>
                <TranscriptRow
                  startMs={s.t0Ms ?? 0}
                  speaker={speaker}
                  words={segmentWords(s)}
                  edited={s.edited}
                  overlap={s.overlap}
                  marks={marksOf.get(s.gid)}
                  playing={v.index === active}
                  activeWordIndex={
                    v.index === active ? activeWord(s, timeMs) : undefined
                  }
                  selected={selected === s.gid}
                  onSelect={() =>
                    setSelected(selected === s.gid ? null : s.gid)
                  }
                />
                {selected === s.gid && (
                  <div className="flex flex-wrap justify-end gap-2 px-2 pt-1 pb-1">
                    {canPlay && s.t0Ms !== null && (
                      <Button
                        icon="play_arrow"
                        className="min-h-ios-target px-4"
                        onClick={() => onPlay(s.t0Ms ?? 0)}
                      >
                        {t("mobile.detail.playFrom", {
                          time: formatClock(s.t0Ms, { pad: true }),
                        })}
                      </Button>
                    )}
                    {!readOnly && (
                      <Button
                        icon="edit"
                        className="min-h-ios-target px-4"
                        onClick={() => {
                          setDraft(s.text);
                          setSaveFailed(false);
                          setEditing(s.gid);
                        }}
                      >
                        {t("mobile.detail.edit")}
                      </Button>
                    )}
                  </div>
                )}
              </>
            )}
          </div>
        );
      })}
    </div>
  );
}
