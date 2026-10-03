// SPDX-License-Identifier: Apache-2.0
// M6: a phone call is active. iOS does not let an app record calls, so say so
// and offer the way that works (speakerphone + Room mode) with the consent
// reminder. Its confirm is what lifts the call block (`callAcknowledged`).
import { Icon, PhoneButton, Sheet } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import type { MeetingLanguage } from "../../bindings";
import { CopyConsentButton } from "./consent-sheet";

export type CallNoticeSheetProps = {
  language: MeetingLanguage;
  open: boolean;
  onCancel: () => void;
  onConfirm: () => void;
};

export function CallNoticeSheet({
  language,
  open,
  onCancel,
  onConfirm,
}: CallNoticeSheetProps) {
  const { t } = useTranslation();
  return (
    <Sheet
      open={open}
      onOpenChange={(o) => !o && onCancel()}
      title={t("mobile.callLimit.title")}
      description={t("mobile.callLimit.body")}
      closeLabel={t("mobile.sheet.close")}
      handleLabel={t("mobile.sheet.handle")}
      footer={
        <>
          <PhoneButton onClick={onConfirm}>
            {t("mobile.callLimit.useRoom")}
          </PhoneButton>
          <PhoneButton variant="secondary" onClick={onCancel}>
            {t("mobile.common.cancel")}
          </PhoneButton>
        </>
      }
    >
      <p className="text-ios-subhead m-0 flex items-start gap-2 text-ink">
        <Icon
          name="speaker_phone"
          size={22}
          className="mt-0.5 size-[1.375rem] shrink-0 text-accent"
        />
        {t("mobile.record.callNotice.steps")}
      </p>
      <p className="text-ios-subhead m-0 mt-3 flex items-start gap-2 text-ink">
        <Icon
          name="record_voice_over"
          size={22}
          className="mt-0.5 size-[1.375rem] shrink-0 text-accent"
        />
        {t("mobile.callLimit.consent")}
      </p>
      <CopyConsentButton language={language} className="mt-4" />
    </Sheet>
  );
}
