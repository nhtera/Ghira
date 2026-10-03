// SPDX-License-Identifier: Apache-2.0
// The audio session was taken (a phone call, another app). Recording never
// resumes by itself: this sheet asks, and stays until the user answers.
import { formatClock } from "@ghi/i18n";
import { Sheet } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import { PhoneButton } from "./phone-button";

export type InterruptionSheetProps = {
  open: boolean;
  call: boolean;
  recordedS: number | null;
  onResume: () => void;
  onStop: () => void;
};

export function InterruptionSheet({
  open,
  call,
  recordedS,
  onResume,
  onStop,
}: InterruptionSheetProps) {
  const { t } = useTranslation();
  const time = formatClock((recordedS ?? 0) * 1000);
  return (
    <Sheet
      open={open}
      onOpenChange={() => {}}
      dismissible={false}
      title={
        call
          ? t("mobile.record.pausedCall.title")
          : t("mobile.record.interrupted.title")
      }
      description={
        call
          ? t("mobile.record.pausedCall.body", { time })
          : t("mobile.record.interrupted.body", { time })
      }
      closeLabel={t("mobile.sheet.close")}
      handleLabel={t("mobile.sheet.handle")}
      footer={
        <>
          <PhoneButton icon="play_arrow" onClick={onResume}>
            {t("mobile.record.resumeRecording")}
          </PhoneButton>
          <PhoneButton variant="secondary" onClick={onStop}>
            {t("mobile.record.stopAndSave")}
          </PhoneButton>
        </>
      }
    />
  );
}
