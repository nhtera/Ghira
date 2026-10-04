// SPDX-License-Identifier: Apache-2.0
// What both enrollment screens show while the passage is read: the level meter,
// the timer and why "Done reading" waits. Plus the banner for a failed try.
import { Banner, cn, Icon } from "@ghi/ui";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { MIN_SECONDS, type EnrollState } from "./use-voice-enroll";

/** Below this the phone hears no speech: the core counts a moment as speech
 * at RMS 0.005 (`SPEECH_RMS`), which the level reports as 0.02 (RMS x 4). */
const QUIET = 0.02;

/** Quiet for this long before the meter says so: gaps between words are not silence. */
export const QUIET_AFTER_MS = 1200;

/** `below` turned into a flag that rises only after it held for `QUIET_AFTER_MS`. */
export function useQuiet(below: boolean) {
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!below) return;
    const id = window.setTimeout(() => setArmed(true), QUIET_AFTER_MS);
    return () => {
      window.clearTimeout(id);
      setArmed(false);
    };
  }, [below]);
  return below && armed;
}

export function EnrollMeter({ state }: { state: EnrollState }) {
  const { t } = useTranslation();
  const level = Math.min(1, Math.max(0, state.level ?? 0));
  const pct = Math.round(level * 100);
  const quiet = useQuiet(level < QUIET);
  const text = quiet ? t("mobile.voice.levelQuiet") : t("mobile.voice.levelGood");
  const seconds = Math.floor(state.seconds);
  const ready = seconds >= MIN_SECONDS;
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <div
          role="meter"
          aria-label={t("mobile.voice.level")}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={pct}
          aria-valuetext={text}
          data-quiet={quiet}
          className="h-2.5 flex-1 overflow-hidden rounded-full bg-line"
        >
          <div
            className={cn("h-full rounded-full transition-[width] duration-100 ease-linear", quiet ? "bg-warn" : "bg-accent")}
            style={{ width: `${pct}%` }}
          />
        </div>
        <Icon name={quiet ? "mic_off" : "graphic_eq"} size={20} className={cn("size-5", quiet ? "text-warn" : "text-accent")} />
      </div>
      <p className={cn("text-ios-footnote m-0 font-medium", quiet ? "text-warn" : "text-muted")}>{text}</p>
      <p className="text-ios-subhead m-0 tabular-nums" data-testid="enroll-timer">
        {t("mobile.voice.reading", { seconds, max: Math.round(state.maxSeconds) })}
      </p>
      {!ready && <p className="text-ios-footnote m-0 text-muted">{t("mobile.voice.waitHint", { min: MIN_SECONDS })}</p>}
    </div>
  );
}

const ERRORS: Record<string, "errorTooShort" | "errorTooQuiet" | "errorMicPermission"> = {
  tooShort: "errorTooShort",
  tooQuiet: "errorTooQuiet",
  micPermission: "errorMicPermission",
};

/** The failure of the last try: what the user can do about it. */
export function EnrollError({ code, skippable = false }: { code: string | null; skippable?: boolean }) {
  const { t } = useTranslation();
  if (!code) return null;
  if (code === "noModel") return <Banner variant="info" title={t("mobile.voice.needsModels")} />;
  return <Banner variant="warning" title={t(`mobile.voice.${ERRORS[code] ?? (skippable ? "error" : "errorRetry")}`)} />;
}
