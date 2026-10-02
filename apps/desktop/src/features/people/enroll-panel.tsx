// SPDX-License-Identifier: Apache-2.0
// The body of "record your voice": the passage, a progress bar with the live
// level, the consent checkbox and the status line. The buttons belong to the
// caller (the onboarding step and the re-record dialog lay them out their way).
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { Icon, LevelMeter, usePlatform } from "@ghi/ui";
import { errorText } from "./error-text";
import type { EnrollState } from "./use-voice-enroll";

/** The locale key of the consent sentence shown: kept with the profile as the consent record. */
export const useConsentKey = () => `onboarding.voice.consent_${usePlatform()}` as const;

const toDb = (level: number | null) => (level != null && level > 0 ? Math.max(-60, 20 * Math.log10(level)) : -60);

/** The voice model isn't there: still downloading, blocked by strict offline, or just not installed. */
export type ModelNote = "downloading" | "offline" | "missing";

export function EnrollPanel({
  state,
  consent,
  onConsent,
  modelNote,
}: {
  state: EnrollState;
  consent: boolean;
  onConsent: (v: boolean) => void;
  /** Why recording can't start yet; null when the voice model is ready. */
  modelNote: ModelNote | null;
}) {
  const { t } = useTranslation();
  const consentId = useId();
  const key = useConsentKey();
  const busy = state.phase !== "idle";
  const percent = Math.min(100, (state.seconds / state.maxSeconds) * 100);
  return (
    <>
      <p lang="vi" className="m-0 rounded-xl bg-surface2 px-5 py-4 font-serif text-[19px] leading-[1.7]">
        {t("onboarding.voice.passage")}
      </p>
      <div
        role="progressbar"
        aria-label={t("onboarding.voice.title")}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={state.phase === "done" ? 100 : Math.round(percent)}
        className="h-2 overflow-hidden rounded bg-sunk"
      >
        <i className="block h-full bg-accent" style={{ width: `${state.phase === "done" ? 100 : percent}%` }} />
      </div>
      {state.phase === "reading" && <LevelMeter db={toDb(state.level)} source="mic" />}
      <div className="flex items-start gap-2.5 text-[13px] leading-normal">
        <input id={consentId} type="checkbox" checked={consent} onChange={(e) => onConsent(e.target.checked)} disabled={busy} className="mt-0.5 size-4 accent-[var(--accent)]" />
        <label htmlFor={consentId}>{t(key)}</label>
      </div>
      {modelNote && (
        <p className="text-small m-0 flex items-start gap-1.5 text-muted">
          <Icon name={modelNote === "downloading" ? "download" : "info"} size={16} className="shrink-0" />
          {modelNote === "downloading" ? t("onboarding.voice.modelWaiting") : modelNote === "offline" ? t("onboarding.voice.modelOffline") : t("people.errors.noModel")}
        </p>
      )}
      <div role="status" className="flex min-h-6 items-center gap-1.5 text-[14px] font-semibold text-accent">
        {state.phase === "reading" && (
          <>
            <Icon name="graphic_eq" size={19} />
            {t("onboarding.voice.reading", { seconds: Math.floor(state.seconds), max: state.maxSeconds })}
          </>
        )}
        {state.phase === "saving" && t("onboarding.voice.saving")}
        {state.phase === "done" && (
          <>
            <Icon name="check_circle" size={19} />
            {t("onboarding.voice.done")}
          </>
        )}
      </div>
      {state.error && (
        <p role="alert" className="text-small m-0 flex items-start gap-2 text-rec-ink">
          <Icon name="error" size={16} className="shrink-0" />
          {errorText(t, state.error)}
        </p>
      )}
    </>
  );
}
