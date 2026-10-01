// SPDX-License-Identifier: Apache-2.0
// Floating panel anchored to a trigger (speaker rename, filters).
import * as RP from "@radix-ui/react-popover";
import type { ReactNode } from "react";
import { cn } from "../utils/cn";

export function Popover({
  trigger,
  children,
  open,
  onOpenChange,
  side = "bottom",
  align = "start",
  className,
  label,
}: {
  trigger: ReactNode;
  children: ReactNode;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  side?: "top" | "bottom" | "left" | "right";
  align?: "start" | "center" | "end";
  className?: string;
  /** Accessible name of the panel. */
  label?: string;
}) {
  return (
    <RP.Root open={open} onOpenChange={onOpenChange}>
      <RP.Trigger asChild>{trigger}</RP.Trigger>
      <RP.Portal>
        <RP.Content
          side={side}
          align={align}
          sideOffset={6}
          collisionPadding={8}
          aria-label={label}
          className={cn("z-50 rounded-panel border border-line2 bg-surface p-3 text-ink shadow-float outline-none", className)}
        >
          {children}
        </RP.Content>
      </RP.Portal>
    </RP.Root>
  );
}

export const PopoverClose = RP.Close;
