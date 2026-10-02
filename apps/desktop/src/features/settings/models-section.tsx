// SPDX-License-Identifier: Apache-2.0
// Settings → Models: what is installed, download / cancel with progress, a
// damaged file is downloaded again. Reuses the onboarding download hook.
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { Button, Icon, ModelRow, cn, type ModelStatus } from "@ghi/ui";
import { APP_NAME, formatBytes } from "@ghi/i18n";
import { loadLicenses } from "../../generated/licenses";
import { useModelDownload, type ModelProgress } from "../onboarding/use-model-download";
import { useOnline } from "../onboarding/models-step";
import { Card, Note, useSettings } from "./parts";

function rowStatus(m: ModelProgress, downloading: boolean): ModelStatus {
  if (m.installed) return "installed";
  if (m.active || (downloading && !m.failed)) return "downloading";
  return "paused";
}

export function ModelsSection() {
  const { t, i18n } = useTranslation();
  const dl = useModelDownload();
  const online = useOnline();
  const { settings } = useSettings();
  const { data: licenses } = useQuery({ queryKey: ["licenses"], queryFn: loadLicenses, staleTime: Infinity });
  const strict = settings?.strictOffline ?? false;
  const canDownload = online && !strict;
  const downloading = dl.phase === "downloading";
  const missing = dl.models.some((m) => !m.installed);

  return (
    <div className="flex max-w-3xl flex-col gap-4">
      <Card title={t("settings.models.title")}>
        {dl.status && <Note icon="info">{t(`settings.models.tier.${dl.status.tier as "light" | "balanced" | "max"}`, { defaultValue: dl.status.tier })}</Note>}
        {strict && <Note icon="cloud_off">{t("settings.models.strictOffline")}</Note>}
        {!strict && !online && <Note icon="wifi_off">{t("onboarding.models.offline", { app: APP_NAME })}</Note>}
        {(downloading || dl.phase === "failed") && (
          <div className="flex flex-col gap-1.5 rounded-xl bg-surface2 px-3.5 py-3">
            <span className={cn("text-small font-medium", dl.phase === "failed" && "text-warn")}>
              {dl.phase === "failed" ? t("download.failed.title", { percent: dl.percent }) : t("settings.models.status.downloading", { percent: dl.percent })}
            </span>
            <div role="progressbar" aria-label={t("settings.models.title")} aria-valuemin={0} aria-valuemax={100} aria-valuenow={dl.percent} className="h-1.5 overflow-hidden rounded-[3px] bg-sunk">
              <i className={cn("block h-full", dl.phase === "failed" ? "bg-warn" : "bg-accent")} style={{ width: `${dl.percent}%` }} />
            </div>
          </div>
        )}
        <div className="flex items-center gap-2">
          {downloading ? (
            <Button onClick={dl.cancel}>{t("common.cancel")}</Button>
          ) : (
            missing && (
              <Button variant="primary" icon="download" disabled={!canDownload} onClick={dl.start}>
                {dl.phase === "failed" ? t("download.resume") : t("settings.models.actions.download")}
              </Button>
            )
          )}
          {!missing && dl.phase === "done" && (
            <span className="text-small inline-flex items-center gap-1.5 text-accent">
              <Icon name="check_circle" size={16} />
              {t("onboarding.models.done")}
            </span>
          )}
        </div>
        <div className="divide-y divide-line overflow-hidden rounded-xl border border-line">
          {dl.models.map((m) => {
            const lic = licenses?.models.find((x) => x.id === m.model.id);
            return (
              <div key={m.model.id}>
                <ModelRow
                  purpose={t(`onboarding.models.roles.${m.model.role as "asr" | "diarization" | "llm" | "embed" | "voice"}`)}
                  name={lic?.name ?? m.model.id}
                  size={formatBytes(m.model.size, i18n.language)}
                  license={lic?.license}
                  status={rowStatus(m, downloading)}
                  progress={m.percent}
                  onDownload={canDownload ? dl.start : undefined}
                  onResume={canDownload ? dl.start : undefined}
                />
                {m.model.damaged && (
                  <div className="flex items-center gap-3 bg-warn-soft px-4 py-2.5">
                    <Icon name="warning" size={18} className="text-warn" />
                    <span className="text-small flex-1">{t("settings.models.damaged")}</span>
                    <Button size="sm" disabled={!canDownload} onClick={dl.start} aria-label={t("settings.models.redownloadFor", { name: lic?.name ?? m.model.id })}>
                      {t("settings.models.redownload")}
                    </Button>
                  </div>
                )}
                {lic && lic.licenseKeys.length === 0 && (
                  <p className="text-small m-0 px-4 pb-2.5 break-all text-muted" data-testid={`model-url-${m.model.id}`}>
                    {t("settings.models.licenseUrl", { license: lic.license, url: lic.url })}
                  </p>
                )}
              </div>
            );
          })}
        </div>
      </Card>
    </div>
  );
}
