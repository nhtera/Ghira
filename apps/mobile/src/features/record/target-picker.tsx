// SPDX-License-Identifier: Apache-2.0
// Where the recording is processed after it stops (M2, and the M1 default): a
// three-way choice with a hint under it. Native radios, 44 pt tall, text and
// icon in every option (never color alone). "My computer" waits for pairing
// (phase 15); cloud is chosen per meeting from the meeting view, so it is
// listed but off here.
import { Icon, cn, type IconName } from "@ghi/ui";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import type { ProcessingTarget } from "../../bindings";

const OPTIONS: {
  value: ProcessingTarget;
  icon: IconName;
  label:
    "mobile.target.phone" | "mobile.target.desktop" | "mobile.target.cloud";
}[] = [
  { value: "phone", icon: "mobile", label: "mobile.target.phone" },
  { value: "desktop", icon: "laptop_mac", label: "mobile.target.desktop" },
  { value: "cloud", icon: "cloud", label: "mobile.target.cloud" },
];

export type TargetPickerProps = {
  value: ProcessingTarget;
  onChange: (target: ProcessingTarget) => void;
  /** Targets that cannot be picked. */
  disabled: readonly ProcessingTarget[];
  /** The hint for the picked target is shown under the control. */
  hint?: boolean;
  className?: string;
};

export function TargetPicker({
  value,
  onChange,
  disabled,
  hint = true,
  className,
}: TargetPickerProps) {
  const { t } = useTranslation();
  const name = useId();
  return (
    <fieldset className={cn("m-0 min-w-0 border-0 p-0", className)}>
      <legend className="text-ios-footnote mb-1.5 p-0 font-medium text-muted">
        {t("mobile.target.finalOn")}
      </legend>
      <div className="grid grid-cols-3 gap-0.5 rounded-(--ios-radius-group) bg-sunk p-0.5">
        {OPTIONS.map((o) => {
          const off = disabled.includes(o.value);
          return (
            <label
              key={o.value}
              className={cn(
                "text-ios-footnote relative flex min-h-ios-target cursor-pointer flex-col items-center justify-center gap-0.5 rounded-[10px] px-1 py-1.5 text-center font-medium text-muted",
                "has-checked:bg-surface has-checked:text-ink has-checked:shadow-sm has-focus-visible:outline-2 has-focus-visible:outline-accent",
                off && "cursor-not-allowed opacity-50",
              )}
            >
              <input
                type="radio"
                name={name}
                value={o.value}
                checked={value === o.value}
                disabled={off}
                onChange={() => onChange(o.value)}
                className="sr-only"
              />
              <Icon name={o.icon} size={20} className="size-5" />
              {t(o.label)}
            </label>
          );
        })}
      </div>
      {hint && value !== "cloud" && (
        <p className="text-ios-footnote m-0 mt-1.5 text-muted">
          {value === "phone"
            ? t("mobile.target.hintPhone")
            : t("mobile.target.hintDesktop", {
                device: t("mobile.target.desktop"),
              })}
        </p>
      )}
    </fieldset>
  );
}
