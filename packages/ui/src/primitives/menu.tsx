// SPDX-License-Identifier: Apache-2.0
// Dropdown menu (row "More" actions, export, merge into…).
import * as RM from "@radix-ui/react-dropdown-menu";
import type { ReactNode } from "react";
import { Icon, type IconName } from "../icons/icon";
import { cn } from "../utils/cn";

export type MenuItem =
  | { kind: "checkbox"; label: string; checked: boolean; onSelect: () => void }
  | { kind?: "item"; label: string; icon?: IconName; onSelect: () => void; danger?: boolean; disabled?: boolean; hint?: string }
  | { kind: "separator" };

export function Menu({ trigger, items, align = "end", label }: { trigger: ReactNode; items: MenuItem[]; align?: "start" | "end"; label?: string }) {
  return (
    <RM.Root modal={false}>
      <RM.Trigger asChild>{trigger}</RM.Trigger>
      <RM.Portal>
        <RM.Content
          align={align}
          sideOffset={4}
          collisionPadding={8}
          aria-label={label}
          className="z-50 min-w-48 rounded-row border border-line2 bg-surface p-1 text-ink shadow-float"
        >
          {items.map((it, i) =>
            it.kind === "separator" ? (
              <RM.Separator key={i} className="my-1 h-px bg-line" />
            ) : it.kind === "checkbox" ? (
              <RM.CheckboxItem
                key={i}
                checked={it.checked}
                onSelect={it.onSelect}
                className="flex h-8 cursor-default items-center gap-2 rounded-seg px-2 text-[13px] outline-none select-none data-[highlighted]:bg-sunk"
              >
                <span className="grid w-4 place-items-center">
                  <RM.ItemIndicator>
                    <Icon name="check" size={16} />
                  </RM.ItemIndicator>
                </span>
                <span className="flex-1">{it.label}</span>
              </RM.CheckboxItem>
            ) : (
              <RM.Item
                key={i}
                disabled={it.disabled}
                onSelect={it.onSelect}
                className={cn(
                  "flex h-8 cursor-default items-center gap-2 rounded-seg px-2 text-[13px] outline-none select-none",
                  "data-[highlighted]:bg-sunk data-[disabled]:opacity-50",
                  it.danger && "text-rec",
                )}
              >
                {it.icon && <Icon name={it.icon} size={16} />}
                <span className="flex-1">{it.label}</span>
                {it.hint && <span className="text-mono text-muted">{it.hint}</span>}
              </RM.Item>
            ),
          )}
        </RM.Content>
      </RM.Portal>
    </RM.Root>
  );
}
