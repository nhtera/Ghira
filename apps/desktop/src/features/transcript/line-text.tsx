// SPDX-License-Identifier: Apache-2.0
// The words of one line (text nodes only, RT-6): karaoke on the word being
// spoken (only while playing, and only when the core timed the words), low
// confidence words dotted, find matches marked. Clicking a word plays from it.
import { Fragment, memo, useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@ghi/ui";
import type { SegmentView } from "../../bindings";
import { usePlayer } from "../../state/player";
import { lineWords, wordIndexAt, type LineWord } from "./logic";

/** How long a click waits for a second one. */
const DOUBLE_CLICK_MS = 250;

/** A find match inside a line: [start, end) and whether it is the current one. */
export type LineRange = readonly [number, number, boolean];

function pieces(w: LineWord, ranges: readonly LineRange[]): { text: string; hit: 0 | 1 | 2 }[] {
  const end = w.offset + w.text.length;
  const out: { text: string; hit: 0 | 1 | 2 }[] = [];
  let at = w.offset;
  for (const [s, e, cur] of ranges) {
    if (e <= at || s >= end) continue;
    const from = Math.max(s, at);
    if (from > at) out.push({ text: w.text.slice(at - w.offset, from - w.offset), hit: 0 });
    const to = Math.min(e, end);
    out.push({ text: w.text.slice(from - w.offset, to - w.offset), hit: cur ? 2 : 1 });
    at = to;
  }
  if (at < end || !out.length) out.push({ text: w.text.slice(at - w.offset), hit: 0 });
  return out;
}

export const LineText = memo(function LineText({ seg, active, ranges }: { seg: SegmentView; active: boolean; ranges: readonly LineRange[] }) {
  const { t } = useTranslation();
  const words = useMemo(() => lineWords(seg), [seg]);
  const timed = words.length > 0 && words.every((w) => w.t0Ms != null);
  const word = usePlayer((s) => (active && s.playing && timed ? wordIndexAt(words, s.currentMs) : -1));

  // A double-click edits the line, so a single click waits to see whether a second one follows.
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const onClick = (e: React.MouseEvent) => {
    clearTimeout(timer.current);
    if (e.detail > 1) return;
    const at = (e.target as HTMLElement).closest<HTMLElement>("[data-t]")?.dataset.t;
    timer.current = setTimeout(() => usePlayer.getState().seek(at != null ? Number(at) : (seg.t0Ms ?? 0), true), DOUBLE_CLICK_MS);
  };

  return (
    // The time button of the group and the line menu are the keyboard routes to "play from here".
    <p onClick={onClick} className="m-0 cursor-pointer font-serif text-transcript text-ink">
      {words.map((w, i) => (
        <Fragment key={i}>
          {i > 0 && " "}
          <span
            data-t={w.t0Ms ?? undefined}
            data-low={w.low ? "true" : undefined}
            data-active={i === word ? "true" : undefined}
            title={w.low ? t("detail.lowConfidence") : undefined}
            className={cn(
              "rounded-[3px]",
              w.low && "underline decoration-warn decoration-dotted decoration-2 underline-offset-4",
              i === word && "font-semibold text-accent underline decoration-accent decoration-2 underline-offset-4",
            )}
          >
            {pieces(w, ranges).map((p, k) =>
              p.hit ? (
                <mark key={k} data-current={p.hit === 2 ? "true" : undefined} className={cn("rounded-[3px] px-px text-ink", p.hit === 2 ? "bg-warn-soft outline-2 outline-warn" : "bg-warn-soft")}>
                  {p.text}
                </mark>
              ) : (
                <Fragment key={k}>{p.text}</Fragment>
              ),
            )}
            {w.low && <span className="sr-only">{` (${t("detail.lowConfidence")})`}</span>}
          </span>
        </Fragment>
      ))}
    </p>
  );
});
