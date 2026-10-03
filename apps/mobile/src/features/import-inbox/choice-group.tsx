// SPDX-License-Identifier: Apache-2.0
import { cn } from "@ghi/ui";

/** A pick-one row of 44 pt buttons (language, target). Disabled options stay visible. */
export function ChoiceGroup<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: { value: T; label: string; disabled?: boolean }[];
  onChange: (v: T) => void;
}) {
  return (
    <div role="group" aria-label={label} className="flex gap-1.5">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          aria-pressed={value === o.value}
          disabled={o.disabled}
          onClick={() => onChange(o.value)}
          className={cn(
            "text-ios-subhead min-h-ios-target flex-1 rounded-(--ios-radius-group) border px-2 font-semibold disabled:opacity-50",
            value === o.value
              ? "border-accent bg-accent-soft text-accent"
              : "border-ctl bg-surface text-ink",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}
