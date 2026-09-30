// SPDX-License-Identifier: Apache-2.0
// Virtualized transcript: only the visible rows are in the DOM, so an
// hour-long meeting (1,500+ lines) scrolls smoothly in the webview.
import { useEffect, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Line } from "./bindings";

// Speakers are shown as color + label, never color alone.
const COLORS = ["#2f6fde", "#d9480f", "#2b8a3e", "#ae3ec9", "#e8590c", "#1098ad", "#c2255c", "#5c940d"];

function stamp(s: number): string {
  const t = Math.floor(s);
  return `${String(Math.floor(t / 60)).padStart(2, "0")}:${String(t % 60).padStart(2, "0")}`;
}

export function SpeakerChip({ speaker }: { speaker: number | null }) {
  if (speaker == null) return <span className="speaker unknown">?</span>;
  const color = COLORS[(speaker - 1) % COLORS.length];
  return (
    <span className="speaker" style={{ borderColor: color, color }} aria-label={`Speaker ${speaker}`}>
      S{speaker}
    </span>
  );
}

export function TranscriptList({ lines, partial }: { lines: Line[]; partial: string }) {
  const parent = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const count = lines.length + (partial ? 1 : 0);
  // The virtualizer returns functions the React Compiler must not memoize.
  // eslint-disable-next-line react-hooks/incompatible-library
  const v = useVirtualizer({
    count,
    getScrollElement: () => parent.current,
    estimateSize: () => 64,
    overscan: 8,
  });

  useEffect(() => {
    if (follow.current && count > 0) v.scrollToIndex(count - 1, { align: "end" });
  }, [count, partial, v]);

  return (
    <div
      className="transcript"
      ref={parent}
      onScroll={(e) => {
        const el = e.currentTarget;
        follow.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
      }}
    >
      {count === 0 && <p className="empty">The transcript appears here while you record.</p>}
      <div style={{ height: v.getTotalSize(), position: "relative" }}>
        {v.getVirtualItems().map((item) => {
          const line = lines[item.index];
          return (
            <div
              key={item.key}
              data-index={item.index}
              ref={v.measureElement}
              className="row"
              style={{ position: "absolute", top: 0, left: 0, right: 0, transform: `translateY(${item.start}px)` }}
            >
              {line ? (
                <>
                  <div className="meta">
                    <SpeakerChip speaker={line.speaker} />
                    <time>{stamp(line.start ?? 0)}</time>
                  </div>
                  <p>{line.text}</p>
                </>
              ) : (
                <p className="partial">{partial}</p>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
