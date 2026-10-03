// SPDX-License-Identifier: Apache-2.0
// The audio bar: play / pause, position, seek and speed.
import { formatClock } from "@ghi/i18n";
import { Icon } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import type { useAudio } from "./use-audio";

export function AudioBar({ audio }: { audio: ReturnType<typeof useAudio> }) {
  const { t } = useTranslation();
  const total = audio.durationMs ?? 0;
  return (
    <div
      role="group"
      aria-label={t("mobile.detail.listen")}
      className="flex flex-wrap items-center gap-x-2 gap-y-1 border-t border-line bg-surface px-2 py-1"
    >
      {audio.failed && (
        <p
          role="status"
          className="text-ios-footnote m-0 w-full px-2 pt-1 text-muted"
        >
          {t("mobile.audio.unavailable")}
        </p>
      )}
      <button
        type="button"
        onClick={() => void audio.toggle()}
        aria-label={
          audio.playing ? t("mobile.audio.pause") : t("mobile.audio.play")
        }
        className="grid min-h-ios-target min-w-ios-target place-items-center rounded-full text-accent"
      >
        <Icon
          name={audio.playing ? "pause" : "play_arrow"}
          size={30}
          className="size-[1.875rem]"
        />
      </button>
      <input
        type="range"
        min={0}
        max={total}
        step={1000}
        value={Math.min(audio.timeMs, total)}
        onChange={(e) => void audio.seek(Number(e.target.value))}
        aria-label={t("mobile.audio.position")}
        aria-valuetext={`${formatClock(audio.timeMs)} / ${formatClock(total)}`}
        className="h-11 min-w-24 flex-1 accent-(--accent)"
      />
      <span className="text-ios-caption1 text-mono tabular-nums text-muted">{`${formatClock(audio.timeMs)} / ${formatClock(total)}`}</span>
      <button
        type="button"
        onClick={audio.cycleSpeed}
        aria-label={t("mobile.audio.speed", { speed: audio.speed })}
        className="text-ios-subhead min-h-ios-target min-w-ios-target rounded-full px-2 font-semibold text-accent"
      >
        {`${audio.speed}×`}
      </button>
    </div>
  );
}
