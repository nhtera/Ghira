// SPDX-License-Identifier: Apache-2.0
// The live transcript: virtualized (a 3-hour meeting has thousands of lines),
// follows the newest line until the user scrolls up, then offers "jump to live".
import { useVirtualizer } from "@tanstack/react-virtual";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, TranscriptLine, wordsFromText } from "@ghi/ui";
import type { LineInfo, SpeakerInfo } from "../../bindings";
import { speakerNumber, useSpeakerLabel } from "../../state/speaker-label";
import { useLive } from "../../state/live";
import { LineSpeakerPicker } from "../speakers";
import { isFollowing } from "./logic";

type Speaker = ReturnType<typeof toSpeaker>;
const toSpeaker = (label: string, s: SpeakerInfo) => ({ label, colorSlot: s.colorSlot, isMe: s.isMe, initial: speakerNumber(s) ?? undefined });

/** One final line; unchanged lines don't re-render as new ones arrive. */
const Row = memo(
  function Row({ line, speaker, marked }: { line: LineInfo; speaker: Speaker | null; marked: boolean }) {
    const { t } = useTranslation();
    const [picking, setPicking] = useState(false);
    const box = useRef<HTMLDivElement>(null);
    // The picker's anchor isn't focusable: put focus back on the line's button.
    const closePicker = () => {
      setPicking(false);
      requestAnimationFrame(() => box.current?.querySelector<HTMLElement>(`button[aria-label="${t("speakers.line.changeSpeaker")}"]`)?.focus());
    };
    // A line can change speaker once it is stored (has a gid) and has one.
    const movable = Boolean(line.gid) && line.speaker != null && !speaker?.isMe;
    return (
      <div ref={box}>
        <TranscriptLine
          startMs={line.t0Ms ?? 0}
          speaker={speaker}
          words={line.words.length ? line.words.map((w) => ({ text: w.text, lowConfidence: w.lowConfidence })) : wordsFromText(line.text)}
          marked={marked}
          onChangeSpeaker={movable ? () => setPicking(true) : undefined}
        />
        {picking && line.speaker != null && <LineSpeakerPicker gid={line.gid} from={line.speaker} onClose={closePicker} />}
      </div>
    );
  },
  (a, b) => a.line === b.line && a.marked === b.marked && a.speaker?.label === b.speaker?.label && a.speaker?.colorSlot === b.speaker?.colorSlot && a.speaker?.initial === b.speaker?.initial,
);

const ESTIMATE_PX = 68;

export function TranscriptView() {
  const { t } = useTranslation();
  const lines = useLive((s) => s.lines);
  const partial = useLive((s) => s.partial);
  const speakers = useLive((s) => s.speakers);
  const marks = useLive((s) => s.marks);
  const labelOf = useSpeakerLabel();
  const words = Object.values(partial).filter(Boolean).join(" ");
  const count = lines.length + (words ? 1 : 0);

  const scrollRef = useRef<HTMLDivElement>(null);
  const [following, setFollowing] = useState(true);
  const virtual = useVirtualizer({
    count,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ESTIMATE_PX,
    overscan: 8,
    initialRect: { width: 600, height: 600 },
  });

  // New line, new words, or a measured row taller than its estimate: stay on
  // the newest while the user hasn't scrolled away.
  const total = virtual.getTotalSize();
  useEffect(() => {
    if (following && count > 0) virtual.scrollToIndex(count - 1, { align: "end" });
  }, [count, words, total, following, virtual]);

  const speakerOf = useMemo(
    () => (id: number | null) => {
      const s = id != null ? speakers[id] : undefined;
      return s ? toSpeaker(labelOf(s), s) : null;
    },
    [speakers, labelOf],
  );
  const markedAt = (l: LineInfo) => marks.some((m) => m >= (l.t0Ms ?? 0) && m <= (l.t1Ms ?? 0));

  return (
    <div className="relative min-h-0 flex-1">
      <div ref={scrollRef} onScroll={(e) => setFollowing(isFollowing(e.currentTarget))} data-testid="transcript-scroll" className="h-full overflow-auto">
        {/* Announcing every line is too noisy: the shell announces new speaker turns. */}
        <ol aria-live="off" style={{ height: virtual.getTotalSize() }} className="relative m-0 w-full list-none p-0">
          {virtual.getVirtualItems().map((v) => {
            const line = lines[v.index];
            return (
              <li key={line ? line.gid || line.t0Ms || v.index : "partial"} data-index={v.index} ref={virtual.measureElement} className="absolute top-0 left-0 w-full pb-1" style={{ transform: `translateY(${v.start}px)` }}>
                {line ? (
                  <Row line={line} speaker={speakerOf(line.speaker)} marked={markedAt(line)} />
                ) : (
                  <TranscriptLine startMs={lines.at(-1)?.t1Ms ?? 0} speaker={null} words={wordsFromText(words)} partial />
                )}
              </li>
            );
          })}
        </ol>
      </div>
      {!following && (
        <Button
          variant="primary"
          size="sm"
          icon="arrow_upward"
          data-testid="jump-to-live"
          onClick={() => {
            setFollowing(true);
            virtual.scrollToIndex(count - 1, { align: "end" });
          }}
          className="absolute right-4 bottom-3 shadow-float [&>svg]:rotate-180"
        >
          {t("live.jumpToLive")}
        </Button>
      )}
    </div>
  );
}
