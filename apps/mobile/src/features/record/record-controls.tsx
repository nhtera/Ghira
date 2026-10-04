// SPDX-License-Identifier: Apache-2.0
// The record screen's round controls, as in the design: Mark (a star) on the
// left, the big ring in the middle (a red dot to record, a red square to stop)
// and Pause / Resume on the right. Idle shows only the ring. The busy and error
// states are @ghi/ui's RecordControl, which already says what is wrong.
import { cn, Icon, RecordControl } from "@ghi/ui";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { RecordState } from "@ghi/ui";

export type RecordControlsProps = {
  state: RecordState;
  capturing: boolean;
  paused: boolean;
  marks: number;
  /** The accessible name of Mark ("Mark, 2 marks"). */
  markLabel: string;
  onStart: () => void;
  onStop: () => void;
  onPause: () => void;
  onResume: () => void;
  onMark: () => void;
  onFix: () => void;
};

/** The two side buttons: 52 pt circles that, like the 44 pt target, do not grow with text. */
const SIDE = "relative grid size-[52px] shrink-0 place-items-center rounded-full border border-ctl bg-surface2 text-ink active:bg-sunk disabled:cursor-not-allowed disabled:opacity-50";

function Ring({ label, onPress, children }: { label: string; onPress: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={onPress}
      className="grid size-[76px] shrink-0 place-items-center rounded-full border-[5px] border-line2 bg-transparent active:opacity-80"
    >
      {children}
    </button>
  );
}

export function RecordControls({ state, capturing, paused, marks, markLabel, onStart, onStop, onPause, onResume, onMark, onFix }: RecordControlsProps) {
  const { t } = useTranslation();
  if (state !== "idle" && state !== "recording" && state !== "paused") {
    return <RecordControl state={state} mode="room" onFix={onFix} className="self-center" />;
  }
  if (state === "idle") {
    return (
      <div role="group" aria-label={t("record.controls")} data-state="idle" className="flex items-center justify-center">
        <Ring label={t("tray.recordRoom")} onPress={onStart}>
          <span aria-hidden="true" className="size-[52px] rounded-full bg-rec" />
        </Ring>
      </div>
    );
  }
  return (
    <div role="group" aria-label={t("record.controls")} data-state={state} className="flex items-center justify-center gap-8">
      <button type="button" aria-label={markLabel} disabled={!capturing} onClick={onMark} className={SIDE}>
        <Icon name="star" size={26} className="size-[1.625rem] text-warn" />
        {marks > 0 && (
          <span aria-hidden="true" className="text-ios-caption2 absolute -top-1 -end-1 grid min-w-5 place-items-center rounded-full bg-warn px-1 font-semibold text-on-accent tabular-nums">
            {marks}
          </span>
        )}
      </button>
      <Ring label={t("live.stop")} onPress={onStop}>
        <span aria-hidden="true" className={cn("size-7 rounded-[6px] bg-rec")} />
      </Ring>
      <button type="button" aria-label={paused ? t("live.resume") : t("live.pause")} onClick={paused ? onResume : onPause} className={SIDE}>
        <Icon name={paused ? "play_arrow" : "pause"} size={28} className="size-7" />
      </button>
    </div>
  );
}
