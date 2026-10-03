// SPDX-License-Identifier: Apache-2.0
// Over the transcript and notes while paused: nothing is recorded, Resume is
// the one action. Speakers and notes are kept underneath.
import { useTranslation } from "react-i18next";
import { Icon } from "@ghi/ui";
import { useAppActions } from "../../shell/actions";

export function PausedOverlay() {
  const { t } = useTranslation();
  const { resumeRecording } = useAppActions();
  return (
    <div data-testid="paused-overlay" role="status" className="absolute inset-0 z-10 grid place-items-center bg-surface/95">
      <div className="flex flex-col items-center gap-2.5 text-center">
        <Icon name="pause_circle" size={40} className="text-muted" />
        <b className="text-[18px]">{t("live.paused.title")}</b>
        <span className="text-[13px] text-muted">{t("live.paused.subtitle")}</span>
        <button type="button" onClick={() => void resumeRecording()} className="mt-1.5 inline-flex h-10 items-center gap-1.5 rounded-panel bg-accent px-[22px] text-[14px] font-semibold text-on-accent hover:brightness-110">
          <Icon name="play_arrow" size={20} />
          {t("live.resume")}
        </button>
      </div>
    </div>
  );
}
