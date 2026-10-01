// SPDX-License-Identifier: Apache-2.0
// Segmented control (one choice of a few): theme, layout, Call/Room.
import * as ToggleGroup from "@radix-ui/react-toggle-group";
import { Icon, type IconName } from "../icons/icon";
import { cn } from "../utils/cn";

export type SegmentOption<T extends string> = { value: T; label: string; icon?: IconName };

export function Segmented<T extends string>({
  value,
  onChange,
  options,
  label,
  className,
}: {
  value: T;
  onChange: (v: T) => void;
  options: SegmentOption<T>[];
  /** Accessible name of the group. */
  label: string;
  className?: string;
}) {
  return (
    <ToggleGroup.Root
      type="single"
      value={value}
      onValueChange={(v) => v && onChange(v as T)}
      aria-label={label}
      className={cn("inline-flex gap-0.5 rounded-ctl border border-ctl p-0.5", className)}
    >
      {options.map((o) => (
        <ToggleGroup.Item
          key={o.value}
          value={o.value}
          className={cn(
            "inline-flex h-6 items-center gap-1 rounded-seg px-2.5 text-[12px] font-medium text-muted",
            "data-[state=on]:bg-accent-soft data-[state=on]:text-accent",
          )}
        >
          {o.icon && <Icon name={o.icon} size={14} />}
          {o.label}
        </ToggleGroup.Item>
      ))}
    </ToggleGroup.Root>
  );
}
