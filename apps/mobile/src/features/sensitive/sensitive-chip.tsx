// SPDX-License-Identifier: Apache-2.0
// "Sensitive" on a library row: icon and word, never color alone.
import { Icon } from "@ghi/ui";
import { useTranslation } from "react-i18next";

export function SensitiveChip() {
  const { t } = useTranslation();
  return (
    <span data-testid="row-sensitive" className="text-ios-footnote inline-flex min-h-6 items-center gap-1 rounded-full bg-warn-soft px-2 font-semibold text-warn">
      <Icon name="visibility_off" size={14} className="size-3.5" />
      {t("mobile.sensitive.chip")}
    </span>
  );
}
