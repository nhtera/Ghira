// SPDX-License-Identifier: Apache-2.0
// "Sensitive · no audio kept": the badge a sensitive meeting wears while it is
// recorded and in its detail. Text and icon, never color alone.
import { Icon, cn } from "@ghi/ui";
import { useTranslation } from "react-i18next";

export function SensitiveBadge({ className }: { className?: string }) {
  const { t } = useTranslation();
  return (
    <span
      data-testid="sensitive-badge"
      className={cn("text-ios-footnote inline-flex min-h-7 items-center gap-1 self-start rounded-full bg-warn-soft px-2.5 font-semibold text-warn", className)}
    >
      <Icon name="visibility_off" size={16} className="size-4" />
      {t("mobile.sensitive.badge")}
    </span>
  );
}
