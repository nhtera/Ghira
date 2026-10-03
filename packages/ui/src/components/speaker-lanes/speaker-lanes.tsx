// SPDX-License-Identifier: Apache-2.0
// Speaker lanes (brief §7): one horizontal row per speaker, segments in the
// speaker color, hatched where people talk over each other. Pure DOM. The
// whole track is a slider: hover shows the time under the pointer, click
// seeks, arrow keys move it (Shift = bigger steps, Home/End).
import { formatClock } from "@ghi/i18n";
import { useState, type KeyboardEvent, type PointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../utils/cn";
import { Avatar } from "../avatar";

export type LaneSpeaker = {
  id: number;
  label: string;
  /** 1..8; 0 is the shared "Others" lane. */
  colorSlot: number;
  /** Text of the small avatar before the label (a number, "+3"); default: the label's first letter. */
  initial?: string;
  isMe?: boolean;
  /** Others: how many voices share the lane ("+3"). */
  count?: number;
};
export type LaneSegment = { speaker: number; t0Ms: number; t1Ms: number };

export type SpeakerLanesProps = {
  speakers: LaneSpeaker[];
  segments: LaneSegment[];
  /** Length of the timeline; while `live` this is "now" and keeps growing. */
  durationMs: number;
  live?: boolean;
  onSeek?: (ms: number) => void;
  /** Show the scrub marker here regardless of the pointer (static previews). */
  scrubMs?: number;
  className?: string;
};

const STEP_MS = 5_000;
const HATCH = "repeating-linear-gradient(135deg, var(--ink) 0 2px, transparent 2px 5px)";

const fill = (slot: number) => (slot > 0 ? `var(--s${slot})` : "var(--muted)");

/** Parts of each speaker's segments that another speaker is also talking over. */
export function overlapsOf(segments: LaneSegment[]): Map<number, Array<[number, number]>> {
  const out = new Map<number, Array<[number, number]>>();
  for (const a of segments) {
    for (const b of segments) {
      if (a.speaker === b.speaker) continue;
      const t0 = Math.max(a.t0Ms, b.t0Ms);
      const t1 = Math.min(a.t1Ms, b.t1Ms);
      if (t1 > t0) out.set(a.speaker, [...(out.get(a.speaker) ?? []), [t0, t1]]);
    }
  }
  return out;
}

export function SpeakerLanes({ speakers, segments, durationMs, live, onSeek, scrubMs, className }: SpeakerLanesProps) {
  const { t } = useTranslation();
  const [hoverMs, setHoverMs] = useState<number | null>(null);
  const [cursorMs, setCursorMs] = useState(0);
  const [focused, setFocused] = useState(false);
  const total = Math.max(1, durationMs);
  const overlaps = overlapsOf(segments);
  const pct = (ms: number) => Math.min(100, Math.max(0, (ms / total) * 100));
  const at = (e: PointerEvent<HTMLElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    return Math.round(Math.min(1, Math.max(0, (e.clientX - r.left) / Math.max(1, r.width))) * total);
  };
  const move = (ms: number) => {
    const next = Math.min(total, Math.max(0, ms));
    setCursorMs(next);
    onSeek?.(next);
  };
  const onKey = (e: KeyboardEvent) => {
    const step = e.shiftKey ? STEP_MS * 6 : STEP_MS;
    const keys: Record<string, number> = { ArrowRight: cursorMs + step, ArrowUp: cursorMs + step, ArrowLeft: cursorMs - step, ArrowDown: cursorMs - step, Home: 0, End: total };
    const next = keys[e.key];
    if (next === undefined) return;
    e.preventDefault();
    move(next);
  };

  const marker = scrubMs ?? hoverMs ?? (focused ? cursorMs : null);

  return (
    <div data-live={live ? "true" : undefined} className={cn("grid grid-cols-[110px_minmax(0,1fr)] items-start gap-x-2.5 gap-y-1.5 rounded-[10px] bg-surface2 px-3 py-2.5", className)}>
      <div className="flex flex-col gap-1.5">
        {speakers.map((s) => (
          <span key={s.id} className="flex h-4 min-w-0 items-center gap-1.5 text-[12px] leading-4 text-muted">
            {s.colorSlot === 0 ? (
              <Avatar kind="group" count={s.count ?? 0} size="sm" className="!size-4 !text-[9px]" />
            ) : (
              <Avatar kind={s.isMe ? "me" : "person"} name={s.label} initial={s.initial} colorSlot={s.colorSlot} size="sm" className="!size-4 !text-[9px]" />
            )}
            <span className="truncate">{s.label}</span>
          </span>
        ))}
      </div>
      <div className="relative">
        <div className="flex flex-col gap-1.5">
          {speakers.map((s) => (
            <div key={s.id} data-lane={s.id} className="relative my-[3px] h-2.5 rounded-[3px] bg-sunk">
              {segments
                .filter((g) => g.speaker === s.id)
                .map((g, i, mine) => {
                  const growing = live && i === mine.length - 1 && g.t1Ms >= durationMs - 1000;
                  return (
                    <i
                      key={`${g.t0Ms}-${i}`}
                      className="absolute inset-y-0 rounded-[3px]"
                      style={{
                        left: `${pct(g.t0Ms)}%`,
                        width: `${Math.max(0.4, pct(g.t1Ms) - pct(g.t0Ms))}%`,
                        background: growing ? `linear-gradient(90deg, ${fill(s.colorSlot)} 70%, var(--surface2))` : fill(s.colorSlot),
                      }}
                    />
                  );
                })}
              {(overlaps.get(s.id) ?? []).map(([t0, t1], i) => (
                <i key={`o${i}`} data-overlap="true" className="absolute inset-y-0 rounded-[3px]" style={{ left: `${pct(t0)}%`, width: `${Math.max(0.4, pct(t1) - pct(t0))}%`, background: HATCH }} />
              ))}
            </div>
          ))}
        </div>
        <div
          role="slider"
          tabIndex={0}
          aria-label={t("live.timeline")}
          aria-orientation="horizontal"
          aria-valuemin={0}
          aria-valuemax={total}
          aria-valuenow={Math.round(cursorMs)}
          aria-valuetext={formatClock(cursorMs, { pad: true })}
          onPointerMove={(e) => setHoverMs(at(e))}
          onPointerLeave={() => setHoverMs(null)}
          onClick={(e) => move(at(e as unknown as PointerEvent<HTMLElement>))}
          onKeyDown={onKey}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          className="absolute -inset-x-0.5 -inset-y-1 cursor-col-resize rounded-seg"
        />
        {marker !== null && (
          <>
            <span aria-hidden="true" data-marker="true" className="pointer-events-none absolute -inset-y-1 w-0.5 bg-ink" style={{ left: `${pct(marker)}%` }} />
            <span
              aria-hidden="true"
              className="pointer-events-none absolute -top-7 -translate-x-1/2 rounded-[4px] bg-toast-bg px-1.5 py-0.5 text-mono text-[10.5px] text-toast-fg"
              style={{ left: `${pct(marker)}%` }}
            >
              {formatClock(marker, { pad: true })}
            </span>
          </>
        )}
      </div>
      <span aria-hidden="true" />
      <div aria-hidden="true" className="flex justify-between text-mono text-[10px] text-muted">
        <span>{formatClock(0, { pad: true })}</span>
        <span>{formatClock(total / 2, { pad: true })}</span>
        <span>{formatClock(total, { pad: true })}</span>
      </div>
    </div>
  );
}
