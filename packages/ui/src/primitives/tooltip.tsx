// SPDX-License-Identifier: Apache-2.0
// Short hint on hover/focus (never the only place information lives).
import * as RT from "@radix-ui/react-tooltip";
import type { ReactNode } from "react";

export const TooltipProvider = RT.Provider;

export function Tooltip({ content, children, side = "top" }: { content: ReactNode; children: ReactNode; side?: "top" | "bottom" | "left" | "right" }) {
  return (
    <RT.Root delayDuration={400}>
      <RT.Trigger asChild>{children}</RT.Trigger>
      <RT.Portal>
        <RT.Content
          side={side}
          sideOffset={6}
          className="z-50 max-w-64 rounded-seg bg-toast-bg px-2 py-1 text-[12px] leading-snug text-toast-fg"
        >
          {content}
        </RT.Content>
      </RT.Portal>
    </RT.Root>
  );
}
