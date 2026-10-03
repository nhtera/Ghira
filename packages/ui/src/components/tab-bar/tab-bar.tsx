// SPDX-License-Identifier: Apache-2.0
// The phone app's tab bar: four tabs, the emphasized one (Record) a filled
// circle. 44 pt targets; the bottom padding is the home-indicator safe area.
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
  /** The primary action's tab (Record): drawn as a filled circle. */
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
      <ul className="m-0 flex list-none p-0">
        {items.map((item) => {
          const selected = item.id === value;
          return (
            <li key={item.id} className="flex-1">
              <button
                type="button"
                onClick={() => onChange(item.id)}
                aria-current={selected ? "page" : undefined}
                data-tab={item.id}
                className={cn(
                  "flex min-h-[var(--ios-tab-h)] w-full min-w-ios-target flex-col items-center justify-center gap-0.5 px-1 py-1 text-[11px] leading-tight font-medium",
                  selected ? "text-accent" : "text-muted",
                )}
              >
                {item.emphasized ? (
                  <span className={cn("grid size-9 place-items-center rounded-full", selected ? "bg-accent text-on-accent" : "bg-accent-soft text-accent")}>
                    <Icon name={selected ? (item.iconSelected ?? item.icon) : item.icon} size={22} />
                  </span>
                ) : (
                  <Icon name={selected ? (item.iconSelected ?? item.icon) : item.icon} size={24} className="size-6" />
                )}
                <span className="max-w-full truncate">{item.label}</span>
              </button>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}
