// SPDX-License-Identifier: Apache-2.0
// The phone app's tab bar, as the design draws it: four equal columns, a 24 px
// icon over a 10.5 px label on one baseline, faint when idle, accent when
// selected, and red for the emphasized one (Record). 44 pt targets; the bottom padding is the home-indicator safe area.
// Labels keep their size at any text scale (the system tab bar does too), so
// four tabs always fit. Icon + text always.
import { Icon, type IconName } from "../../icons/icon";
import { cn } from "../../utils/cn";

export type TabBarItem = {
  id: string;
  label: string;
  icon: IconName;
  /** Filled glyph used while the tab is selected. */
  iconSelected?: IconName;
  /** The primary action's tab (Record): red while selected, on the same line as the rest. */
  emphasized?: boolean;
};

export type TabBarProps = {
  items: TabBarItem[];
  value: string;
  onChange: (id: string) => void;
  /** Accessible name of the navigation landmark. */
  label: string;
  className?: string;
};

export function TabBar({ items, value, onChange, label, className }: TabBarProps) {
  return (
    <nav aria-label={label} className={cn("border-t border-line bg-surface pb-safe", className)}>
      <ul className="m-0 grid list-none grid-cols-4 p-0 pt-2 pb-1">
        {items.map((item) => {
          const selected = item.id === value;
          return (
            <li key={item.id} className="min-w-0">
              <button
                type="button"
                onClick={() => onChange(item.id)}
                aria-current={selected ? "page" : undefined}
                data-tab={item.id}
                className={cn(
                  "flex min-h-ios-target w-full min-w-ios-target flex-col items-center justify-start gap-0.5 px-1 text-[10.5px] leading-[1.3] font-medium",
                  selected ? (item.emphasized ? "text-rec-ink" : "text-accent") : "text-faint",
                )}
              >
                <Icon name={selected ? (item.iconSelected ?? item.icon) : item.icon} size={24} className="size-6" />
                <span className="max-w-full truncate">{item.label}</span>
              </button>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}
