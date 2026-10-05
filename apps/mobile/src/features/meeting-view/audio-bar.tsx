// SPDX-License-Identifier: Apache-2.0
// The audio bar, as in the design: a round play button, the waveform coloured by
// who is speaking (what has played is solid, the rest faded; drag it to seek)
// and "16:28 / 42:07 · 1×" under it. The waveform is the seek control: a tap
// jumps, a drag scrubs (pointer events, 44 pt tall; iOS WebKit only moves a
// range input by its thumb). A visually hidden range input carries the
// position for VoiceOver and the keyboard.
import { formatClock } from "@ghi/i18n";
import { Icon } from "@ghi/ui";
import { useMemo, useRef, type PointerEvent } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingSpeaker, SegmentView, Waveform } from "../../bindings";
import type { useAudio } from "./use-audio";
import { waveBars } from "./wave-model";

const BARS = 56;

export type AudioBarProps = {
  audio: ReturnType<typeof useAudio>;
  wave: Waveform | null;
  segments: readonly SegmentView[];
  speakers: readonly MeetingSpeaker[];
};

export function AudioBar({ audio, wave, segments, speakers }: AudioBarProps) {
  const { t } = useTranslation();
  const total = audio.durationMs ?? 0;
  const bars = useMemo(() => waveBars(wave, total, segments, speakers, BARS), [wave, total, segments, speakers]);
  const dragging = useRef(false);
  const seekAt = (e: PointerEvent<HTMLDivElement>) => {
    if (total <= 0) return;
    const box = e.currentTarget.getBoundingClientRect();
    const fraction = box.width > 0 ? (e.clientX - box.left) / box.width : 0;
    void audio.seek(Math.round(Math.min(1, Math.max(0, fraction)) * total));
  };
  const played = total > 0 ? (Math.min(audio.timeMs, total) / total) * BARS : 0;
  return (
    <div role="group" aria-label={t("mobile.detail.listen")} className="flex items-center gap-3 border-t border-line bg-surface px-5 pt-2.5 pb-3">
      <button
        type="button"
        onClick={() => void audio.toggle()}
        aria-label={audio.playing ? t("mobile.audio.pause") : t("mobile.audio.play")}
        className="grid size-11 shrink-0 place-items-center rounded-full bg-accent text-on-accent active:brightness-90"
      >
        <Icon name={audio.playing ? "pause_fill" : "play_arrow_fill"} size={26} className="size-[1.625rem]" />
      </button>
      <div className="flex min-w-0 flex-1 flex-col">
        {audio.failed && (
          <p role="status" className="text-ios-footnote m-0 text-muted">
            {t("mobile.audio.unavailable")}
          </p>
        )}
        <div
          data-testid="audio-seek"
          onPointerDown={(e) => {
            dragging.current = true;
            try {
              e.currentTarget.setPointerCapture(e.pointerId);
            } catch {
              /* no capture (tests): the drag still follows pointermove over the bar */
            }
            seekAt(e);
          }}
          onPointerMove={(e) => dragging.current && seekAt(e)}
          onPointerUp={() => (dragging.current = false)}
          onPointerCancel={() => (dragging.current = false)}
          className="relative flex h-11 cursor-pointer touch-none items-center rounded-sm has-focus-visible:outline-2 has-focus-visible:outline-offset-2 has-focus-visible:outline-accent"
        >
          <div aria-hidden="true" data-testid="audio-wave" className="flex h-8 w-full items-center gap-[2px]">
            {bars.map((b, i) => (
              <span
                key={i}
                className="min-w-0 flex-1 rounded-[1px]"
                style={{
                  height: `${Math.round(b.height * 100)}%`,
                  background: b.slot > 0 ? `var(--s${b.slot})` : "var(--faint)",
                  opacity: i + 0.5 < played ? 1 : 0.4,
                }}
              />
            ))}
          </div>
          <input
            type="range"
            min={0}
            max={total}
            step={1000}
            value={Math.min(audio.timeMs, total)}
            onChange={(e) => void audio.seek(Number(e.target.value))}
            aria-label={t("mobile.audio.position")}
            aria-valuetext={`${formatClock(audio.timeMs)} / ${formatClock(total)}`}
            className="sr-only"
          />
        </div>
        <div className="text-ios-caption1 flex items-center font-mono text-muted tabular-nums">
          <span>{`${formatClock(audio.timeMs, { pad: true })} / ${formatClock(total, { pad: true })}`}</span>
          <button
            type="button"
            onClick={audio.cycleSpeed}
            aria-label={t("mobile.audio.speed", { speed: audio.speed })}
            className="-my-2.5 ms-1 min-h-ios-target min-w-ios-target px-1 font-semibold text-accent"
          >
            {`· ${audio.speed}×`}
          </button>
        </div>
      </div>
    </div>
  );
}
