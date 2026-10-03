// SPDX-License-Identifier: Apache-2.0
// Record control for the phone (platform "ios"): the thumb-zone variant. One
// 72 px primary button (record circle that morphs into the stop square), 56 px
// pause/resume beside it, labels under the buttons, the clock above. Room mode
// only on the phone, so there is no mode menu.
import { formatClock } from "@ghi/i18n";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Icon } from "../../icons/icon";
import { cn } from "../../utils/cn";
import type { RecordControlProps } from "./record-control";

const MOTION = "transition-[background-color,border-radius] duration-(--motion-base) ease-out";

function Labelled({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="flex flex-col items-center gap-1.5">
      {children}
      {/* The button carries the name; the caption is for sighted users only. */}
      <span aria-hidden="true" className="text-ios-footnote font-medium text-ink">
        {label}
      </span>
    </span>
  );
}

export function RecordControlThumb({ state, mode, elapsedMs = 0, onStart, onPause, onResume, onStop, onFix, className }: RecordControlProps) {
  const { t } = useTranslation();
  const clock = formatClock(elapsedMs);

  if (state === "starting" || state === "stopping") {
    const starting = state === "starting";
    return (
      <span
        role="status"
        aria-busy="true"
        data-state={state}
        className={cn(
          "text-ios-callout inline-flex min-h-14 items-center gap-2 rounded-full px-5 font-semibold",
          starting ? "bg-sunk text-muted" : "bg-rec-soft text-rec-ink",
          className,
        )}
      >
        <Icon name="progress_activity" size={22} className="size-[1.375rem] animate-spin motion-reduce:animate-none" />
        {t(starting ? "record.checkingMic" : "record.saving")}
      </span>
    );
  }

  if (state === "error") {
    return (
      <div role="alert" data-state="error" className={cn("text-ios-callout flex min-h-14 items-center gap-2 rounded-(--ios-radius-group) border border-warn bg-warn-soft py-1 ps-4 pe-1 font-semibold text-warn", className)}>
        <Icon name="error" size={22} className="size-[1.375rem] shrink-0" />
        <span className="min-w-0 flex-1">{t("record.micUnavailable")}</span>
        <button type="button" onClick={onFix} className="min-h-ios-target rounded-full px-3 underline underline-offset-2">
          {t("common.fix")}
        </button>
      </div>
    );
  }

  const idle = state === "idle";
  const paused = state === "paused";
  return (
    <div role="group" data-state={state} data-mode={mode} aria-label={t("record.controls")} className={cn("flex flex-col items-center gap-4", className)}>
      {!idle && <span className="text-ios-title1 font-mono tabular-nums">{clock}</span>}
      <div className="flex items-start justify-center gap-8">
        {!idle && (
          <Labelled label={paused ? t("live.resume") : t("live.pause")}>
            <button
              type="button"
              onClick={paused ? onResume : onPause}
              aria-label={paused ? t("live.resume") : t("live.pause")}
              className="grid size-14 place-items-center rounded-full border border-ctl bg-surface text-ink active:bg-sunk"
            >
              <Icon name={paused ? "play_arrow" : "pause"} size={28} />
            </button>
          </Labelled>
        )}
        <Labelled label={idle ? t(mode === "call" ? "tray.recordCall" : "tray.recordRoom") : t("live.stop")}>
          <button
            type="button"
            onClick={idle ? () => onStart?.(mode) : onStop}
            aria-label={idle ? t(mode === "call" ? "tray.recordCall" : "tray.recordRoom") : t("live.stop")}
            className={cn("grid size-[72px] place-items-center rounded-full text-on-accent active:brightness-90", MOTION, idle ? "bg-accent" : "bg-rec")}
          >
            <span aria-hidden="true" className={cn("size-7 bg-on-accent transition-[border-radius] duration-(--motion-base) ease-out", idle ? "rounded-full" : "rounded-[5px]")} />
          </button>
        </Labelled>
      </div>
    </div>
  );
}
