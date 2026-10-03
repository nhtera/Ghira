// SPDX-License-Identifier: Apache-2.0
// M1 step 6: the speech models (about 877 MB) download once, over Wi-Fi unless
// the user says otherwise this time. Progress, a stopped download and "waiting
// for Wi-Fi" are states of this screen; "later" leaves a record-only app.
import { formatBytes } from "@ghi/i18n";
import { Banner, Icon, cn } from "@ghi/ui";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { MobileModelItem, MobileModelsStatus } from "../../bindings";
import { ipc } from "../../ipc";
import { PhoneButton } from "../record/phone-button";
import { applyItem, modelsView } from "./models-model";
import { StepLayout } from "./step-layout";

const ROLE = {
  asr: "mobile.onboarding.models.role.asr",
  diarization: "mobile.onboarding.models.role.diarization",
  voice: "mobile.onboarding.models.role.voice",
} as const;

function ModelLine({ item }: { item: MobileModelItem }) {
  const { t, i18n } = useTranslation();
  const percent = item.sizeBytes
    ? Math.round(((item.receivedBytes ?? 0) / item.sizeBytes) * 100)
    : 0;
  const state: Record<MobileModelItem["state"], string> = {
    missing: t("mobile.onboarding.models.state.missing"),
    downloading: t("mobile.onboarding.models.state.downloading", { percent }),
    ready: t("mobile.onboarding.models.state.ready"),
    waitingForWifi: t("mobile.onboarding.models.state.waitingForWifi"),
    failed: t("mobile.onboarding.models.state.failed"),
  };
  const icon = {
    missing: "download",
    downloading: "download",
    ready: "check_circle",
    waitingForWifi: "wifi_off",
    failed: "error",
  } as const;
  return (
    <li data-state={item.state} className="flex flex-col gap-1.5 px-4 py-3">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3">
        <span className="text-ios-body">{t(ROLE[item.role])}</span>
        <span className="text-ios-footnote text-muted">
          {formatBytes(item.sizeBytes, i18n.language)}
        </span>
      </div>
      <span
        className={cn(
          "text-ios-footnote inline-flex items-center gap-1 font-medium",
          item.state === "failed"
            ? "text-warn"
            : item.state === "ready"
              ? "text-accent"
              : "text-muted",
        )}
      >
        <Icon name={icon[item.state]} size={16} className="size-4" />
        {state[item.state]}
      </span>
      {item.state === "downloading" && (
        <span
          role="progressbar"
          aria-label={t(ROLE[item.role])}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={percent}
          className="block h-1.5 overflow-hidden rounded-[3px] bg-sunk"
        >
          <i
            className="block h-full bg-accent"
            style={{ width: `${percent}%` }}
          />
        </span>
      )}
    </li>
  );
}

export function ModelsStep({ onNext }: { onNext: () => void }) {
  const { t, i18n } = useTranslation();
  const [status, setStatus] = useState<MobileModelsStatus | null>(null);
  // The download command was refused or died; a failed model also shows in the status.
  const [refused, setRefused] = useState(false);
  const [busy, setBusy] = useState(false);
  const [recordOnly, setRecordOnly] = useState(false);

  useEffect(() => {
    let alive = true;
    let off: (() => void) | undefined;
    void (async () => {
      const u = await ipc.onMobileEvent((e) => {
        if (e.type !== "modelDownload") return;
        setStatus((s) => (s ? applyItem(s, e.item) : s));
      });
      if (!alive) return u();
      off = u;
      const [r, tier] = await Promise.all([ipc.commands.modelsStatus(), ipc.commands.deviceTier()]);
      if (!alive) return;
      if (r.status === "ok") setStatus(r.data);
      if (tier.status === "ok") setRecordOnly(tier.data.tier === "recordOnly");
    })();
    return () => {
      alive = false;
      off?.();
    };
  }, []);

  const download = useCallback(async (wifiOnly: boolean) => {
    setRefused(false);
    setBusy(true);
    const r = await ipc.commands.modelsDownload(wifiOnly);
    setBusy(false);
    if (r.status === "error") setRefused(true);
  }, []);

  if (!status) return null;
  const view = modelsView(status);
  const size = formatBytes(view.remainingBytes, i18n.language);
  const working = busy || view.downloading;
  const failed = refused || view.failed;

  return (
    <StepLayout
      icon="download"
      title={t("mobile.onboarding.models.title")}
      subtitle={t("mobile.onboarding.models.body", { size })}
      footer={
        view.allReady ? (
          <PhoneButton onClick={onNext}>
            {t("mobile.common.continue")}
          </PhoneButton>
        ) : (
          <>
            <PhoneButton
              disabled={working}
              onClick={() => void download(status.wifiOnly)}
            >
              {failed
                ? t("mobile.onboarding.models.retry")
                : t("mobile.onboarding.models.download", { size })}
            </PhoneButton>
            {view.waitingForWifi && (
              <PhoneButton
                variant="secondary"
                disabled={working}
                onClick={() => void download(false)}
              >
                {t("mobile.onboarding.models.cellular")}
              </PhoneButton>
            )}
            <PhoneButton variant="ghost" onClick={onNext}>
              {t("mobile.onboarding.models.later")}
            </PhoneButton>
          </>
        )
      }
    >
      <div className="flex flex-col gap-3">
        {recordOnly && <Banner variant="info" icon="info" title={t("mobile.onboarding.models.optional")} />}
        {failed && (
          <Banner
            variant="warning"
            title={t("mobile.onboarding.models.error")}
          />
        )}
        {view.waitingForWifi && !failed && (
          <Banner
            variant="info"
            icon="wifi_off"
            title={t("mobile.onboarding.models.wifiWaiting")}
          />
        )}
        {view.allReady && (
          <Banner
            variant="info"
            icon="check_circle"
            title={t("mobile.onboarding.models.ready")}
          />
        )}
        <ul className="m-0 list-none divide-y divide-line overflow-hidden rounded-(--ios-radius-group) bg-surface p-0">
          {status.items.map((item) => (
            <ModelLine key={item.id} item={item} />
          ))}
        </ul>
        {status.wifiOnly && !view.allReady && (
          <p className="text-ios-footnote m-0 flex items-center gap-1.5 text-muted">
            <Icon name="wifi" size={16} className="size-4 shrink-0" />
            {t("mobile.onboarding.models.wifi")}
          </p>
        )}
      </div>
    </StepLayout>
  );
}
