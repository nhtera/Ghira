// SPDX-License-Identifier: Apache-2.0
// Input level meter (Mic / System). The state shows as icon + note as well as
// bar color. Width eases 100 ms linear; under reduced motion the bar freezes
// to a per-state width that changes only when the state does.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Icon } from "../../icons/icon";
import { cn } from "../../utils/cn";

export type LevelSource = "mic" | "system";
export type LevelState = "silent" | "normal" | "clipping" | "noDevice";

export type LevelMeterProps = {
  /** Peak level in dBFS (<= 0). null: the device is missing. */
  db: number | null;
  source: LevelSource;
  className?: string;
};

const FLOOR_DB = -60;
const SILENT_DB = -50;
const CLIP_DB = -1;
// Fixed widths used when motion is reduced (design: 2 / 62 / 100 %).
const FROZEN: Record<LevelState, number> = { silent: 2, normal: 62, clipping: 100, noDevice: 0 };

export function levelState(db: number | null): LevelState {
  if (db === null) return "noDevice";
  if (db >= CLIP_DB) return "clipping";
  if (db < SILENT_DB) return "silent";
  return "normal";
}

function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(() => typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches);
  useEffect(() => {
    if (typeof matchMedia !== "function") return;
    const mq = matchMedia("(prefers-reduced-motion: reduce)");
    const on = () => setReduced(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  return reduced;
}

export function LevelMeter({ db, source, className }: LevelMeterProps) {
  const { t } = useTranslation();
  const reduced = useReducedMotion();
  const state = levelState(db);
  const clamped = db === null ? FLOOR_DB : Math.min(0, Math.max(FLOOR_DB, db));
  const width = reduced ? FROZEN[state] : Math.max(state === "silent" ? 2 : 0, ((clamped - FLOOR_DB) / -FLOOR_DB) * 100);
  const name = source === "mic" ? t("live.sourceMic") : t("live.sourceSystem");
  const note: string = {
    silent: String(t("meter.silent")),
    normal: "",
    clipping: String(t("meter.clipping")),
    noDevice: String(source === "mic" ? t("meter.noMic") : t("meter.noSystem")),
  }[state];
  const iconName = state === "noDevice" ? (source === "mic" ? "mic_off" : "volume_off") : source === "mic" ? "mic" : "volume_up";

  return (
    <div data-state={state} className={cn("flex items-center gap-2 text-[12.5px]", className)}>
      <Icon
        name={iconName}
        size={17}
        className={state === "clipping" ? "text-warn" : state === "noDevice" ? "text-rec-ink" : "text-muted"}
      />
      <span className="w-[4.25rem] shrink-0 font-semibold whitespace-nowrap text-muted">{name}</span>
      <div
        role="meter"
        aria-label={name}
        aria-valuemin={FLOOR_DB}
        aria-valuemax={0}
        // No device: the floor, with the value text saying why.
        aria-valuenow={db === null ? FLOOR_DB : Math.round(clamped)}
        aria-valuetext={note || String(t("meter.normal"))}
        className={cn(
          "h-1.5 min-w-16 flex-1 overflow-hidden rounded-[3px] bg-sunk",
          state === "noDevice" && "border border-dashed border-line2 bg-transparent",
        )}
      >
        <i
          className={cn(
            "block h-full ease-linear",
            state === "clipping" ? "bg-warn" : "bg-accent",
            "transition-[width] duration-100 motion-reduce:transition-none",
          )}
          style={{ width: `${width}%` }}
        />
      </div>
      <span
        className={cn(
          "w-28 shrink-0 text-[11.5px] leading-tight",
          state === "clipping" && "text-warn",
          state === "noDevice" && "text-rec-ink",
          state === "silent" && "text-muted",
        )}
      >
        {note}
      </span>
    </div>
  );
}
