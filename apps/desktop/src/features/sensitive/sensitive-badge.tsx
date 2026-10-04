// SPDX-License-Identifier: Apache-2.0
// "Sensitive · no audio kept": the badge a sensitive meeting wears on the live
// screen and in its header. Text and icon, never color alone.
import { Icon, cn } from "@ghi/ui";
import { useTranslation } from "react-i18next";

export function SensitiveBadge({ className }: { className?: string }) {
  const { t } = useTranslation();
  return (
    <span
      data-testid="sensitive-badge"
      title={t("sensitive.tip")}
      className={cn("inline-flex h-6 flex-none items-center gap-1 rounded-full bg-warn-soft px-2 text-[12px] font-semibold whitespace-nowrap text-warn", className)}
    >
      <Icon name="visibility_off" size={14} />
      {t("sensitive.badge")}
    </span>
  );
}
