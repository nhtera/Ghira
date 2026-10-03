// SPDX-License-Identifier: Apache-2.0
// Record control (brief §7): Idle (split Call/Room) · Starting · Recording ·
// Paused · Stopping · Error. `state` is the core SessionState subset the
// control cares about plus "error" (permission/device problem).
import { formatClock } from "@ghi/i18n";
import { useTranslation } from "react-i18next";
import { Icon } from "../../icons/icon";
import { Menu } from "../../primitives/menu";
import { useAppPlatform } from "../../platform/platform";
import { cn } from "../../utils/cn";
import { RecordControlThumb } from "./record-control-thumb";

export type RecordState = "idle" | "starting" | "recording" | "paused" | "stopping" | "error";
export type RecordMode = "call" | "room";

export type RecordControlProps = {
  state: RecordState;
  mode: RecordMode;
  /** Time recorded so far (recording, paused). */
  elapsedMs?: number;
  onStart?: (mode: RecordMode) => void;
  onPause?: () => void;
  onResume?: () => void;
  onStop?: () => void;
  /** Idle: the user picked the other mode from the caret menu. */
  onModeChange?: (mode: RecordMode) => void;
  /** Error: open the fix (permission settings, device picker). */
  onFix?: () => void;
  className?: string;
};

const BTN = "inline-flex h-9 items-center gap-2 px-3.5 text-[13px] font-semibold whitespace-nowrap";
const MOTION = "transition-[background-color,border-radius] duration-(--motion-base) ease-out";

/** The record glyph: a circle that becomes a rounded square while recording (200 ms). */
function RecordShape({ recording }: { recording: boolean }) {
  return <span aria-hidden="true" className={cn("size-3 bg-on-accent transition-[border-radius] duration-(--motion-base) ease-out", recording ? "rounded-[3px]" : "rounded-full")} />;
}

export function RecordControl({ state, mode, elapsedMs = 0, onStart, onPause, onResume, onStop, onModeChange, onFix, className }: RecordControlProps) {
  const { t } = useTranslation();
  const ios = useAppPlatform() === "ios";
  const clock = formatClock(elapsedMs, { pad: true });

  // The phone gets the thumb-zone variant (large buttons, labels under them).
  if (ios) return <RecordControlThumb {...{ state, mode, elapsedMs, onStart, onPause, onResume, onStop, onModeChange, onFix, className }} />;

  if (state === "starting" || state === "stopping") {
    const starting = state === "starting";
    return (
      <span
        role="status"
        aria-busy="true"
        data-state={state}
        className={cn(BTN, "rounded-ctl", starting ? "bg-sunk text-muted" : "bg-rec-soft text-rec-ink", className)}
      >
        <Icon name="progress_activity" size={18} className="animate-spin motion-reduce:animate-none" />
        {t(starting ? "record.checkingMic" : "record.saving")}
      </span>
    );
  }

  if (state === "error") {
    return (
      <div role="alert" data-state="error" className={cn("inline-flex h-9 items-center gap-2 rounded-ctl border border-warn bg-warn-soft pr-1 pl-3 text-[13px] font-semibold text-warn", className)}>
        <Icon name="error" size={18} />
        {t("record.micUnavailable")}
        <button type="button" onClick={onFix} className="h-7 rounded-seg px-2 underline underline-offset-2">
          {t("common.fix")}
        </button>
      </div>
    );
  }

  // idle / recording / paused share one main button so the circle can morph
  // into the stop square (200 ms) instead of remounting.
  const idle = state === "idle";
  const paused = state === "paused";
  return (
    <div role="group" data-state={state} data-mode={mode} aria-label={t("record.controls")} className={cn("inline-flex items-center", idle ? "gap-px" : "gap-1.5", className)}>
      <button
        type="button"
        onClick={idle ? () => onStart?.(mode) : onStop}
        className={cn(BTN, MOTION, "text-on-accent hover:brightness-110", idle ? "rounded-l-ctl rounded-r-[2px] bg-accent" : "rounded-ctl bg-rec")}
      >
        <RecordShape recording={!idle} />
        {idle ? t(mode === "call" ? "tray.recordCall" : "tray.recordRoom") : t("live.stop")}
        {!idle && <span className="text-mono font-medium">{clock}</span>}
      </button>
      {idle && (
        <Menu
          align="end"
          label={t("record.chooseMode")}
          trigger={
            <button type="button" aria-label={t("record.chooseMode")} className="inline-flex h-9 w-8 items-center justify-center rounded-l-[2px] rounded-r-ctl bg-accent text-on-accent hover:brightness-110">
              <Icon name="expand_more" size={18} />
            </button>
          }
          items={[
            { label: t("record.mode.call"), icon: "videocam", onSelect: () => onModeChange?.("call") },
            { label: t("record.mode.room"), icon: "groups", onSelect: () => onModeChange?.("room") },
          ]}
        />
      )}
      {paused && (
        <button type="button" onClick={onResume} className={cn(BTN, MOTION, "rounded-ctl bg-accent text-on-accent hover:brightness-110")}>
          <Icon name="play_arrow" size={18} />
          {t("live.resume")}
        </button>
      )}
      {state === "recording" && (
        <button type="button" onClick={onPause} className={cn(BTN, MOTION, "rounded-ctl border border-ctl bg-surface text-ink hover:bg-surface2")}>
          <Icon name="pause" size={18} />
          {t("live.pause")}
        </button>
      )}
    </div>
  );
}
