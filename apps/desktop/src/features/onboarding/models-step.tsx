// SPDX-License-Identifier: Apache-2.0
// D1 step 3: hardware tier, model rows, download progress. Never blocks:
// Continue is always there, and "Record now, process later" is the answer to
// no internet, a failed download, or strict offline.
import { useEffect, useRef, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { Button, Icon, ModelRow, cn, useToast, usePlatform, type ModelStatus } from "@ghi/ui";
import { formatBytes } from "@ghi/i18n";
import { Notice, StepActions, StepFrame, type StepNav } from "./step-frame";
import type { ModelDownloadState, ModelProgress } from "./use-model-download";

const TIERS = ["light", "balanced", "max"] as const;

const subscribeOnline = (cb: () => void) => {
  window.addEventListener("online", cb);
  window.addEventListener("offline", cb);
  return () => {
    window.removeEventListener("online", cb);
    window.removeEventListener("offline", cb);
  };
};
export const useOnline = () => useSyncExternalStore(subscribeOnline, () => navigator.onLine, () => true);

function rowStatus(m: ModelProgress, downloading: boolean): ModelStatus {
  if (m.installed) return "installed";
  if (m.active || (downloading && !m.failed)) return "downloading";
  return "paused";
}

export function ModelsStep({ nav, dl, strictOffline }: { nav: StepNav; dl: ModelDownloadState; strictOffline: boolean }) {
  const { t, i18n } = useTranslation();
  const context = usePlatform();
  const { show } = useToast();
  const online = useOnline();
  const canDownload = online && !strictOffline;

  // The design starts the download as the step opens (sizes are shown first,
  // and the user can cancel or just continue). Once per visit.
  const auto = useRef(false);
  useEffect(() => {
    if (!auto.current && dl.phase === "idle" && canDownload) {
      auto.current = true;
      dl.start();
    }
  }, [dl, canDownload]);

  const recordLater = () => {
    show({ title: t("download.deferToast") });
    nav.next();
  };
  const downloading = dl.phase === "downloading";
  const failed = dl.phase === "failed";
  const tier = dl.status?.tier;

  return (
    <StepFrame title={t("onboarding.models.title")} body={t("onboarding.models.body", { context })}>
      <ul aria-label={t("settings.models.presetTitle")} className="m-0 grid list-none grid-cols-3 gap-2 p-0">
        {TIERS.map((id) => {
          const on = tier === id;
          return (
            <li key={id} aria-current={on || undefined} className={cn("flex min-w-0 flex-col gap-1 rounded-xl border-[1.5px] px-3.5 py-3 break-words", on ? "border-accent bg-accent-soft" : "border-line2 bg-surface")}>
              <b className="text-[14px]">{t(`onboarding.models.presets.${id}`)}</b>
              <span className="text-small text-muted">{on ? t("onboarding.models.tierDetail", { download: formatBytes(dl.totalBytes, i18n.language) }) : ""}</span>
              {on && <span className="text-small font-semibold text-accent">{t("onboarding.models.recommended")}</span>}
            </li>
          );
        })}
      </ul>

      <div className="flex flex-col gap-2 rounded-xl bg-surface2 px-4 py-3.5">
        <div className="flex items-center gap-2 text-[13px] font-medium">
          <Icon name={dl.phase === "done" ? "check_circle" : failed ? "warning" : "download"} size={18} className={failed ? "text-warn" : "text-accent"} />
          <span>
            {dl.phase === "done"
              ? t("onboarding.models.done")
              : failed
                ? t("download.failed.title", { percent: dl.percent })
                : downloading
                  ? dl.minutesLeft != null
                    ? t("onboarding.models.downloading", { percent: dl.percent, minutes: dl.minutesLeft })
                    : t("settings.models.status.downloading", { percent: dl.percent })
                  : t("download.waiting")}
          </span>
        </div>
        <span role="status" className="sr-only">
          {dl.phase === "done" ? t("onboarding.models.done") : failed ? t("download.failed.body") : downloading ? t("onboarding.models.started") : ""}
        </span>
        <div role="progressbar" aria-label={t("onboarding.models.title")} aria-valuemin={0} aria-valuemax={100} aria-valuenow={dl.percent} className="h-1.5 overflow-hidden rounded-[3px] bg-sunk">
          <i className={cn("block h-full", failed ? "bg-warn" : "bg-accent")} style={{ width: `${dl.percent}%` }} />
        </div>
        {failed && <span className="text-[12.5px] text-warn">{t("download.failed.body")}</span>}
        {strictOffline && <Notice icon="cloud_off">{t("onboarding.models.strictOffline")}</Notice>}
        {!strictOffline && !online && <Notice icon="wifi_off">{t("onboarding.models.offline")}</Notice>}
        {dl.phase !== "done" && (
          <div className="flex flex-wrap items-center gap-2">
            {failed && canDownload && (
              <Button size="md" variant="primary" icon="refresh" onClick={dl.start}>
                {t("download.resume")}
              </Button>
            )}
            {downloading && (
              <Button size="sm" onClick={dl.cancel}>
                {t("common.cancel")}
              </Button>
            )}
            {!downloading && canDownload && !failed && (
              <Button size="sm" onClick={dl.start}>
                {t("settings.models.actions.download")}
              </Button>
            )}
            {(failed || !canDownload) && (
              <Button size="md" onClick={recordLater}>
                {t("download.recordLater")}
              </Button>
            )}
          </div>
        )}
      </div>

      <div className="divide-y divide-line overflow-hidden rounded-xl border border-line">
        {dl.models.map((m) => (
          <ModelRow
            key={m.model.id}
            purpose={t(`onboarding.models.roles.${m.model.role as "asr" | "diarization" | "llm" | "embed" | "voice"}`)}
            name={m.model.id}
            size={formatBytes(m.model.size, i18n.language)}
            status={rowStatus(m, downloading)}
            progress={m.percent}
            onResume={canDownload ? dl.start : undefined}
          />
        ))}
      </div>

      <StepActions nav={nav} />
    </StepFrame>
  );
}
