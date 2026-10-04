// SPDX-License-Identifier: Apache-2.0
// The record screen's "More": the actions that are not Mark, Pause or Stop.
// Sensitive mode (one way: part of the audio is gone) and discarding the last
// minutes. Each asks again before it removes anything.
import { PhoneButton, Sheet } from "@ghi/ui";
import { useTranslation } from "react-i18next";

/** The spans offered, in seconds. */
export const DISCARD_SECONDS = [60, 300, 600] as const;

export type MoreSheetProps = {
  open: boolean;
  onClose: () => void;
  sensitive: boolean;
  /** A live transcript is being made (sensitive mode needs it). */
  canSensitive: boolean;
  onSensitive: () => void;
  onDiscard: (seconds: number) => void;
};

export function MoreSheet({ open, onClose, sensitive, canSensitive, onSensitive, onDiscard }: MoreSheetProps) {
  const { t } = useTranslation();
  return (
    <Sheet open={open} onOpenChange={(o) => !o && onClose()} title={t("mobile.record.more")} closeLabel={t("mobile.sheet.close")} handleLabel={t("mobile.sheet.handle")}>
      <div className="flex flex-col gap-2">
        {sensitive ? (
          <PhoneButton variant="secondary" icon="check" disabled>
            {`${t("mobile.sensitive.label")} · ${t("mobile.sensitive.stays")}`}
          </PhoneButton>
        ) : (
          <PhoneButton variant="secondary" icon="visibility_off" disabled={!canSensitive} onClick={onSensitive}>
            {t("mobile.sensitive.makeIt")}
          </PhoneButton>
        )}
        {DISCARD_SECONDS.map((s) => (
          <PhoneButton key={s} variant="secondary" icon="delete" onClick={() => onDiscard(s)}>
            {t("mobile.record.discard.menu", { count: s / 60 })}
          </PhoneButton>
        ))}
      </div>
    </Sheet>
  );
}
