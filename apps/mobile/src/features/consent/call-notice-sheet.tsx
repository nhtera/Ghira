// SPDX-License-Identifier: Apache-2.0
// M6: a phone call is active. iOS does not let an app record calls, so say so
// and offer the way that works (speakerphone + Room mode) with the consent
// reminder. Its confirm is what lifts the call block (`callAcknowledged`).
import { Icon, PhoneButton, Sheet } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import type { MeetingLanguage } from "../../bindings";
import { SensitiveRow } from "../sensitive";
import { CopyConsentButton } from "./consent-sheet";

export type CallNoticeSheetProps = {
  language: MeetingLanguage;
  open: boolean;
  onCancel: () => void;
  onConfirm: () => void;
  /** The "Sensitive meeting" choice for this recording (none: not offered). */
  sensitive?: { checked: boolean; onChange: (on: boolean) => void };
};

export function CallNoticeSheet({
  language,
  open,
  onCancel,
  onConfirm,
  sensitive,
}: CallNoticeSheetProps) {
  const { t } = useTranslation();
  return (
    <Sheet
      open={open}
      onOpenChange={(o) => !o && onCancel()}
      icon="phone_disabled"
      title={t("mobile.callLimit.title")}
      description={t("mobile.callLimit.body")}
      strongDescription
      closeButton={false}
      tall
      closeLabel={t("mobile.sheet.close")}
      handleLabel={t("mobile.sheet.handle")}
      footer={
        <>
          <PhoneButton icon="speaker_phone" size="small" onClick={onConfirm}>
            {t("mobile.callLimit.useRoom")}
          </PhoneButton>
          <PhoneButton variant="ghost" size="compact" onClick={onCancel}>
            {t("mobile.common.cancel")}
          </PhoneButton>
        </>
      }
    >
      <div className="flex flex-col items-start rounded-xl bg-sunk px-3 pt-2.5 pb-0.5 text-muted">
        <p className="text-ios-subhead m-0 flex items-start gap-2.5">
          <Icon name="campaign" size={22} className="size-[1.375rem] shrink-0 text-accent" />
          {t("mobile.callLimit.consent")}
        </p>
        <CopyConsentButton language={language} compact className="-mb-0.5 ms-8" />
      </div>
      {sensitive && (
        <div className="mt-1">
          <SensitiveRow stacked checked={sensitive.checked} onChange={sensitive.onChange} />
        </div>
      )}
    </Sheet>
  );
}
