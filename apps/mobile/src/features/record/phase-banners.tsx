// SPDX-License-Identifier: Apache-2.0
// What the phase and the shell events ask the user to know while the screen is
// open: loading, locked, catching up (with a %), too hot, record-only (and why),
// muffled sound, plus the saved and error notices. One capped, scrolling region
// so a pile of banners at 200% text never pushes the controls off the screen;
// it is not rendered at all when there is nothing to say.
import { formatClock } from "@ghi/i18n";
import { Banner } from "@ghi/ui";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { RecordOnlyReason } from "../../bindings";
import { catchUpPercent, hasSession, type RecordModel } from "./model";
import type { BannerError } from "./use-record";

export type PhaseBannersProps = {
  model: RecordModel;
  /** The device is below the live tier (idle too: the transcript will not come). */
  recordOnlyDevice: boolean;
  saved?: { onOpen: () => void; onDismiss: () => void };
  error?: { kind: BannerError; onDismiss: () => void };
  onDownloadModels: () => void;
};

export function PhaseBanners({ model, recordOnlyDevice, saved, error, onDownloadModels }: PhaseBannersProps) {
  const { t } = useTranslation();
  const { phase } = model;
  const dismiss = t("mobile.banner.dismiss");
  const items: ReactNode[] = [];

  if (phase === "loading" && hasSession(model)) {
    items.push(
      <Banner key="loading" variant="info" icon="hourglass_top" title={t("mobile.record.loading.title")}>
        {t("mobile.record.loading.body")}
      </Banner>,
    );
  }
  if (phase === "locked") {
    items.push(
      <Banner key="locked" variant="info" icon="lock" title={t("mobile.record.locked.title")}>
        {t("mobile.record.locked.body")}
      </Banner>,
    );
  }
  if (phase === "catchingUp") {
    items.push(
      <Banner key="catching" variant="info" icon="sync" title={t("mobile.record.catchingUp.title", { percent: catchUpPercent(model) })}>
        {t("mobile.record.catchingUp.body", { time: formatClock(Math.max(0, model.backlogS) * 1000) })}
      </Banner>,
    );
  }
  if (phase === "hot") {
    items.push(
      <Banner key="hot" variant="warning" icon="thermostat" title={t("mobile.record.hot.title")}>
        {t("mobile.record.hot.body")}
      </Banner>,
    );
  }
  const idle = phase === "idle" || phase === "done";
  if (phase === "recordOnly" || (recordOnlyDevice && idle)) {
    const reason: RecordOnlyReason | null = model.recordOnlyReason ?? (recordOnlyDevice ? "deviceTier" : null);
    items.push(
      <Banner
        key="recordOnly"
        variant="info"
        icon="mic"
        title={phase === "recordOnly" ? t("mobile.record.recordOnly.title") : t("mobile.record.recordOnlyIdle")}
        action={reason === "modelsMissing" ? { label: t("mobile.record.recordOnlyWhy.modelsMissingAction"), onPress: onDownloadModels } : undefined}
      >
        {reason ? t(`mobile.record.recordOnlyWhy.${reason}`) : t("mobile.record.recordOnly.body")}
      </Banner>,
    );
  }
  if (model.pocket && !idle) {
    items.push(
      <Banner key="pocket" variant="warning" icon="pan_tool" title={t("mobile.record.pocket.title")}>
        {t("mobile.record.pocket.hint")}
      </Banner>,
    );
  }
  if (saved) {
    items.push(
      <Banner
        key="saved"
        variant="info"
        icon="check_circle"
        title={t("mobile.record.saved.title")}
        action={{ label: t("mobile.record.saved.action"), onPress: saved.onOpen }}
        onDismiss={saved.onDismiss}
        dismissLabel={dismiss}
      />,
    );
  }
  if (error) {
    items.push(<Banner key="error" variant="warning" title={t(`mobile.record.error.${error.kind}`)} onDismiss={error.onDismiss} dismissLabel={dismiss} />);
  }

  if (items.length === 0) return null;
  return (
    <div tabIndex={0} role="region" aria-label={t("mobile.record.title")} className="flex max-h-[18dvh] shrink-0 flex-col gap-2 overflow-y-auto">
      {items}
    </div>
  );
}
