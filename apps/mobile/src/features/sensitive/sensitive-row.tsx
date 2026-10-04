// SPDX-License-Identifier: Apache-2.0
// The "Sensitive meeting" switch with its one-line promise, used before
// recording (arms the next recording) and on a stored meeting.
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { Switch } from "../settings/controls";

export function SensitiveRow({
  checked,
  onChange,
  disabled,
  hint,
}: {
  checked: boolean;
  onChange: (on: boolean) => void;
  disabled?: boolean;
  /** Replaces the promise (why the switch is off-limits). */
  hint?: string;
}) {
  const { t } = useTranslation();
  const id = useId();
  return (
    <div className="flex items-center justify-between gap-3" data-testid="sensitive-row">
      <div className="min-w-0">
        <p id={id} className="text-ios-subhead m-0">
          {t("mobile.sensitive.label")}
        </p>
        <p className="text-ios-footnote m-0 text-muted">{hint ?? t("mobile.sensitive.hint")}</p>
      </div>
      <Switch checked={checked} onChange={onChange} labelledBy={id} disabled={disabled} />
    </div>
  );
}
