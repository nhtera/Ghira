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
      closeLabel={t("mobile.sheet.close")}
      handleLabel={t("mobile.sheet.handle")}
      footer={
        <>
          <PhoneButton icon="speaker_phone" onClick={onConfirm}>
            {t("mobile.callLimit.useRoom")}
          </PhoneButton>
          <PhoneButton variant="ghost" onClick={onCancel}>
            {t("mobile.common.cancel")}
          </PhoneButton>
        </>
      }
    >
      <div className="flex flex-col gap-3 rounded-(--ios-radius-group) bg-sunk px-4 py-3">
        <p className="text-ios-callout m-0 flex items-start gap-3 text-ink">
          <Icon name="speaker_phone" size={24} className="mt-0.5 size-6 shrink-0 text-accent" />
          {t("mobile.record.callNotice.steps")}
        </p>
        <p className="text-ios-callout m-0 flex items-start gap-3 text-ink">
          <Icon name="campaign" size={24} className="mt-0.5 size-6 shrink-0 text-accent" />
          {t("mobile.callLimit.consent")}
        </p>
      </div>
      <CopyConsentButton language={language} className="mt-3" />
      {sensitive && (
        <div className="mt-3 rounded-(--ios-radius-group) bg-sunk px-4 py-3">
          <SensitiveRow checked={sensitive.checked} onChange={sensitive.onChange} />
        </div>
      )}
    </Sheet>
  );
}
