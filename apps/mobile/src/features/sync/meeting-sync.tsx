// SPDX-License-Identifier: Apache-2.0
// Sync in the meeting view (M4): "Refining on <computer>" with a way to take
// the job back, and a conflict copy to keep or dismiss.
import { Banner, Icon, Sheet } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import { Btn } from "../settings/controls";

export function RefiningBanner({ device, onProcessHere }: { device: string; onProcessHere: () => void }) {
  const { t } = useTranslation();
  return (
    <Banner
      variant="info"
      icon="laptop_mac"
      className="mx-4 mb-2"
      title={t("mobile.sync.refiningOn", { device })}
      action={{ label: t("mobile.sync.processHere"), onPress: onProcessHere }}
    >
      {t("mobile.sync.refiningHint")}
    </Banner>
  );
}

export function ProcessHereSheet({
  open,
  device,
  busy,
  error,
  onCancel,
  onConfirm,
}: {
  open: boolean;
  device: string;
  busy: boolean;
  error: string | null;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { t } = useTranslation();
  return (
    <Sheet
      open={open}
      onOpenChange={(o) => !o && onCancel()}
      title={t("mobile.sync.lease.confirmTitle")}
      description={t("mobile.sync.lease.confirmBody", { device })}
      closeLabel={t("mobile.sheet.close")}
      handleLabel={t("mobile.sheet.handle")}
      dismissible={!busy}
      footer={
        <div className="flex flex-col gap-2">
          {error && (
            <p role="alert" className="text-ios-footnote m-0 text-rec-ink">
              {t("mobile.sync.lease.failed")}
            </p>
          )}
          <Btn tone="primary" disabled={busy} onClick={onConfirm}>
            {t("mobile.sync.lease.confirm")}
          </Btn>
          <Btn disabled={busy} onClick={onCancel}>
            {t("mobile.common.cancel")}
          </Btn>
        </div>
      }
    />
  );
}

export function ConflictBanner({
  device,
  text,
  busy,
  error,
  onUse,
  onDismiss,
}: {
  device: string;
  text: string;
  busy: boolean;
  error: string | null;
  onUse: () => void;
  onDismiss: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div role="group" aria-label={t("mobile.sync.editedOn", { device })} data-testid="conflict-banner" className="mx-4 mb-2 flex flex-col gap-2 rounded-(--ios-radius-group) bg-warn-soft p-3.5 text-ink">
      <p className="text-ios-subhead m-0 flex items-start gap-2.5 font-semibold">
        <Icon name="edit" size={20} className="mt-0.5 size-5 shrink-0 text-warn" />
        {t("mobile.sync.editedOn", { device })}
      </p>
      <div>
        <p className="text-ios-caption1 m-0 text-muted">{t("mobile.sync.conflict.label")}</p>
        {/* Meeting text from another device: a text node only. */}
        <p className="text-ios-subhead m-0 break-words select-text">{text}</p>
      </div>
      {error && (
        <p role="alert" className="text-ios-footnote m-0 text-rec-ink">
          {t("mobile.sync.conflict.failed")}
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <Btn tone="primary" disabled={busy} onClick={onUse}>
          {t("mobile.sync.useThis")}
        </Btn>
        <Btn disabled={busy} onClick={onDismiss}>
          {t("mobile.sync.dismiss")}
        </Btn>
      </div>
    </div>
  );
}
