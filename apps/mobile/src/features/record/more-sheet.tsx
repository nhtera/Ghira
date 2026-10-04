// SPDX-License-Identifier: Apache-2.0
// The record screen's "More": the actions that are not Mark, Pause or Stop.
// Sensitive mode (one way: part of the audio is gone) and discarding the last
// minutes. Each asks again before it removes anything. One grouped list, like
// the design's share sheet: icon and label on a row, a hairline between rows,
// the destructive ones in the recording red.
import { cn, Icon, Sheet, type IconName } from "@ghi/ui";
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

function Row({ icon, danger, disabled, onPress, children }: { icon: IconName; danger?: boolean; disabled?: boolean; onPress?: () => void; children: string }) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onPress}
      className={cn(
        "text-ios-body flex min-h-14 w-full items-center gap-3 border-t border-line px-4 text-start first:border-t-0 active:bg-sunk disabled:cursor-default",
        danger ? "text-rec-ink" : "text-ink",
        disabled && "text-muted",
      )}
    >
      <Icon name={icon} size={22} className={cn("size-[1.375rem] shrink-0", !danger && "text-muted")} />
      <span className="min-w-0 flex-1">{children}</span>
    </button>
  );
}

export function MoreSheet({ open, onClose, sensitive, canSensitive, onSensitive, onDiscard }: MoreSheetProps) {
  const { t } = useTranslation();
  return (
    <Sheet open={open} onOpenChange={(o) => !o && onClose()} title={t("mobile.record.more")} closeLabel={t("mobile.sheet.close")} handleLabel={t("mobile.sheet.handle")}>
      <div className="overflow-hidden rounded-(--ios-radius-group) bg-surface2">
        {sensitive ? (
          <Row icon="check" disabled>
            {`${t("mobile.sensitive.label")} · ${t("mobile.sensitive.stays")}`}
          </Row>
        ) : (
          <Row icon="visibility_off" disabled={!canSensitive} onPress={onSensitive}>
            {t("mobile.sensitive.makeIt")}
          </Row>
        )}
        {DISCARD_SECONDS.map((s) => (
          <Row key={s} icon="delete" danger onPress={() => onDiscard(s)}>
            {t("mobile.record.discard.menu", { count: s / 60 })}
          </Row>
        ))}
      </div>
    </Sheet>
  );
}
