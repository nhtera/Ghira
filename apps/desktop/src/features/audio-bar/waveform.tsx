// SPDX-License-Identifier: Apache-2.0
// The audio bar's waveform: loudness bars colored by who speaks, the played
// part solid, the playhead, click/drag to seek. A slider for the keyboard
// (arrows ±5 s, Home/End). Colors come from the CSS tokens at draw time, so
// both themes (and a theme switch) are right.
import { formatClock } from "@ghi/i18n";
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingSpeaker, SegmentView, Waveform as WaveformData } from "../../bindings";
import { usePlayer } from "../../state/player";
import { columnSlots, reducePeaks, timeAt } from "./waveform-math";

const BAR = 2;
const GAP = 1;
const HEIGHT = 44;
const STEP_MS = 5000;

type Colors = { slots: string[]; idle: string; accent: string };

function readColors(el: Element): Colors {
  const css = getComputedStyle(el);
  const v = (n: string, fallback: string) => css.getPropertyValue(n).trim() || fallback;
  return { slots: [v("--muted", "#888"), ...Array.from({ length: 8 }, (_, i) => v(`--s${i + 1}`, "#888"))], idle: v("--line2", "#ccc"), accent: v("--accent", "#0a0") };
}

/** Re-reads the colors when the theme changes (attribute on <html>, or the OS scheme). */
function useColors(canvas: React.RefObject<HTMLCanvasElement | null>): number {
  const [version, setVersion] = useState(0);
  useEffect(() => {
    const bump = () => setVersion((v) => v + 1);
    const mo = new MutationObserver(bump);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["class", "data-theme", "style"] });
    const mq = window.matchMedia?.("(prefers-color-scheme: dark)");
    mq?.addEventListener?.("change", bump);
    return () => {
      mo.disconnect();
      mq?.removeEventListener?.("change", bump);
    };
  }, [canvas]);
  return version;
}

export function WaveformSlider({
  data,
  segments,
  speakers,
  durationMs,
  seekTo,
}: {
  data: WaveformData | undefined;
  segments: readonly SegmentView[];
  speakers: readonly MeetingSpeaker[];
  durationMs: number;
  /** Seek to a time, keeping play/pause as it is. */
  seekTo: (ms: number) => void;
}) {
  const { t } = useTranslation();
  const currentMs = usePlayer((s) => s.currentMs);
  const box = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [width, setWidth] = useState(0);
  const dragging = useRef(false);
  const theme = useColors(canvas);
  // Computed styles are read once per theme, not per frame.
  const colorsRef = useRef<{ theme: number; colors: Colors } | null>(null);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    setWidth(el.clientWidth);
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const cols = Math.floor(width / (BAR + GAP));
  const heights = useMemo(() => (data ? reducePeaks(data.peaks, data.perSecond, durationMs, cols) : new Uint8Array(0)), [data, durationMs, cols]);
  const slots = useMemo(() => columnSlots(segments, speakers, durationMs, cols), [segments, speakers, durationMs, cols]);

  useEffect(() => {
    const c = canvas.current;
    const ctx = c?.getContext?.("2d");
    if (!c || !ctx || !width) return;
    const dpr = window.devicePixelRatio || 1;
    if (c.width !== Math.round(width * dpr)) {
      c.width = Math.round(width * dpr);
      c.height = Math.round(HEIGHT * dpr);
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, HEIGHT);
    if (colorsRef.current?.theme !== theme) colorsRef.current = { theme, colors: readColors(c) };
    const colors = colorsRef.current.colors;
    const played = durationMs ? (currentMs / durationMs) * cols : 0;
    for (let i = 0; i < heights.length; i++) {
      const h = Math.max(2, (heights[i]! / 255) * (HEIGHT - 6));
      ctx.globalAlpha = i < played ? 1 : 0.38;
      ctx.fillStyle = slots[i] ? colors.slots[slots[i]!]! : colors.idle;
      ctx.fillRect(i * (BAR + GAP), (HEIGHT - h) / 2, BAR, h);
    }
    ctx.globalAlpha = 1;
    ctx.fillStyle = colors.accent;
    ctx.fillRect(Math.min(width - 2, (played / Math.max(1, cols)) * width), 0, 2, HEIGHT);
    // `theme` re-runs the draw after a theme switch.
  }, [heights, slots, width, cols, currentMs, durationMs, theme]);

  const at = (e: PointerEvent) => timeAt(e.clientX - (box.current?.getBoundingClientRect().left ?? 0), box.current?.clientWidth ?? 0, durationMs);
  const onKey = (e: KeyboardEvent) => {
    const cur = usePlayer.getState().currentMs;
    const to = e.key === "ArrowRight" || e.key === "ArrowUp" ? cur + STEP_MS : e.key === "ArrowLeft" || e.key === "ArrowDown" ? cur - STEP_MS : e.key === "Home" ? 0 : e.key === "End" ? durationMs : null;
    if (to == null) return;
    e.preventDefault();
    seekTo(Math.min(durationMs, Math.max(0, to)));
  };

  return (
    <div
      ref={box}
      role="slider"
      tabIndex={0}
      aria-label={t("audio.position")}
      aria-valuemin={0}
      aria-valuemax={Math.round(durationMs / 1000)}
      aria-valuenow={Math.round(currentMs / 1000)}
      aria-valuetext={`${formatClock(currentMs)} / ${formatClock(durationMs)}`}
      onKeyDown={onKey}
      onPointerDown={(e) => {
        dragging.current = true;
        e.currentTarget.setPointerCapture?.(e.pointerId);
        seekTo(at(e));
      }}
      onPointerMove={(e) => dragging.current && seekTo(at(e))}
      onPointerUp={() => (dragging.current = false)}
      onPointerCancel={() => (dragging.current = false)}
      data-testid="waveform"
      className="relative h-11 min-w-0 flex-1 cursor-pointer touch-none rounded-seg focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
    >
      <canvas ref={canvas} aria-hidden="true" style={{ width: "100%", height: HEIGHT }} className="block" />
    </div>
  );
}
