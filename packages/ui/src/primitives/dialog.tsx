// SPDX-License-Identifier: Apache-2.0
// Modal dialog. mac: a sheet attached under the title bar; Windows: a
// centered dialog (design notes #10). Escape and the scrim close it unless
// `dismissible` is false (consent must be an explicit choice).
import * as RD from "@radix-ui/react-dialog";
import type { ReactNode } from "react";
import { usePlatform } from "../platform/platform";
import { cn } from "../utils/cn";

export type DialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: ReactNode;
  description?: ReactNode;
  children?: ReactNode;
  /** Buttons, right-aligned (primary last on mac, first on Windows is the caller's call). */
  footer?: ReactNode;
  /** false: Escape and outside clicks do nothing. */
  dismissible?: boolean;
  width?: number;
  /** `sheet` (default): a mac sheet under the title bar, centred on Windows. `center`: centred on both. */
  placement?: "sheet" | "center";
};

export function Dialog({ open, onOpenChange, title, description, children, footer, dismissible = true, width = 480, placement = "sheet" }: DialogProps) {
  const sheet = usePlatform() === "mac" && placement === "sheet";
  const block = (e: Event) => {
    if (!dismissible) e.preventDefault();
  };
  return (
    <RD.Root open={open} onOpenChange={onOpenChange}>
      <RD.Portal>
        <RD.Overlay className="fixed inset-0 z-40 bg-[var(--scrim)]" />
        <RD.Content
          onEscapeKeyDown={block}
          onPointerDownOutside={block}
          onInteractOutside={block}
          // No description: tell Radix so (an explicit undefined), rather than
          // repeating the title as one.
          {...(description ? {} : { "aria-describedby": undefined })}
          style={{ width }}
          className={cn(
            "fixed left-1/2 z-50 flex max-h-[85vh] max-w-[calc(100vw-32px)] -translate-x-1/2 flex-col gap-3 overflow-auto",
            "border border-line2 bg-surface p-5 text-ink shadow-float outline-none",
            sheet ? "top-0 rounded-b-dialog border-t-0" : "top-1/2 -translate-y-1/2 rounded-dialog",
          )}
          data-shape={sheet ? "sheet" : "dialog"}
        >
          <RD.Title className="text-heading m-0">{title}</RD.Title>
          {description && <RD.Description className="text-body m-0 text-muted">{description}</RD.Description>}
          {children}
          {footer && <div className="mt-1 flex justify-end gap-2">{footer}</div>}
        </RD.Content>
      </RD.Portal>
    </RD.Root>
  );
}

export const DialogClose = RD.Close;
