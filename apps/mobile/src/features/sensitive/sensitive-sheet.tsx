// SPDX-License-Identifier: Apache-2.0
// Turning sensitive mode on deletes audio (what was recorded so far goes when
// the recording stops; a stored meeting loses it now), so it asks first.
import { PhoneButton, Sheet } from "@ghi/ui";
import { useTranslation } from "react-i18next";

export function SensitiveSheet({
  open,
  recording,
  onCancel,
  onConfirm,
}: {
  open: boolean;
  /** The meeting is being recorded (it can't go back), else it is stored. */
  recording: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { t } = useTranslation();
  return (
    <Sheet
      open={open}
      onOpenChange={(o) => !o && onCancel()}
      title={t("mobile.sensitive.confirmTitle")}
      description={recording ? t("mobile.sensitive.confirmLive") : t("mobile.sensitive.confirmStored")}
      closeLabel={t("mobile.sheet.close")}
      handleLabel={t("mobile.sheet.handle")}
      footer={
        <>
          <PhoneButton onClick={onConfirm}>{t("mobile.sensitive.confirm")}</PhoneButton>
          <PhoneButton variant="secondary" onClick={onCancel}>
            {t("mobile.common.cancel")}
          </PhoneButton>
        </>
      }
    />
  );
}
