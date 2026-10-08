// SPDX-License-Identifier: Apache-2.0
// A pick-one list drawn like the app's other controls (the secondary button's
// border, radius, hover and focus ring, an app chevron) over the native
// <select>, so the menu itself stays the platform's (keyboard, VoiceOver,
// type-ahead).
import { Icon } from "../icons/icon";
import { cn } from "../utils/cn";

export type SelectOption = {
  value: string;
  label: string;
  disabled?: boolean;
};

export type SelectSize = "sm" | "md";

export type SelectProps = {
  value: string;
  onChange: (value: string) => void;
  options: SelectOption[];
  /** The accessible name; leave it out when a <label htmlFor={id}> names it. */
  label?: string;
  id?: string;
  disabled?: boolean;
  /** `md` 32 px (settings rows), `sm` 28 px (beside a segmented control). */
  size?: SelectSize;
  className?: string;
  testId?: string;
};

const SIZE: Record<SelectSize, string> = {
  sm: "h-7 pl-2.5 pr-7 text-[12.5px]",
  md: "h-8 pl-3 pr-8 text-[13.5px]",
};

export function Select({ value, onChange, options, label, id, disabled, size = "md", className, testId }: SelectProps) {
  return (
    <span className={cn("relative inline-flex min-w-0 items-center", className)}>
      <select
        id={id}
        aria-label={label}
        data-testid={testId}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
        className={cn(
          "w-full min-w-0 cursor-pointer appearance-none truncate rounded-ctl border border-ctl bg-surface font-medium text-ink",
          "hover:bg-surface2 focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent",
          "disabled:cursor-default disabled:opacity-55 disabled:hover:bg-surface",
          SIZE[size],
        )}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value} disabled={o.disabled}>
            {o.label}
          </option>
        ))}
      </select>
      <Icon name="expand_more" size={size === "sm" ? 16 : 18} className={cn("pointer-events-none absolute text-muted", size === "sm" ? "right-1.5" : "right-2")} />
    </span>
  );
}
