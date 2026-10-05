// SPDX-License-Identifier: Apache-2.0
// Bottom sheet (iOS): Radix Dialog anchored to the bottom edge with two
// detents, medium (about half the screen) and large. The handle toggles the
// detent on tap and follows a vertical swipe: up expands, down shrinks, and
// down from medium dismisses. Title and body scroll; the handle and footer stay
// pinned, and a sheet whose content does not fit medium opens at large. Focus
// is trapped, Escape and the scrim close it unless `dismissible` is false, and
// reduced motion drops the slide.
import * as RD from "@radix-ui/react-dialog";
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { Icon, type IconName } from "../icons/icon";
import { cn } from "../utils/cn";

export type SheetDetent = "medium" | "large";

export type SheetProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: ReactNode;
  /** A glyph above the title (decorative; the title says it). */
  icon?: IconName;
  description?: ReactNode;
  /** The description in ink rather than muted (a message to read, not a caption). */
  strongDescription?: boolean;
  children?: ReactNode;
  /** Pinned under the scrolling content (primary actions). */
  footer?: ReactNode;
  /** The detent it opens at. */
  detent?: SheetDetent;
  onDetentChange?: (detent: SheetDetent) => void;
  /** Accessible names for the close button and the drag handle. */
  closeLabel: string;
  handleLabel: string;
  /** false: Escape, the scrim and swiping down do nothing (consent). */
  dismissible?: boolean;
  /** false: no close X (the sheet has its own Cancel; swiping down still closes it). */
  closeButton?: boolean;
  /** A taller medium detent (62% instead of 55%) for a sheet whose content is just over half. */
  tall?: boolean;
};

/** A swipe shorter than this (px) counts as a tap. */
const SWIPE = 24;

export function Sheet({ open, onOpenChange, dismissible = true, ...body }: SheetProps) {
  const block = (e: Event) => {
    if (!dismissible) e.preventDefault();
  };
  return (
    <RD.Root open={open} onOpenChange={onOpenChange}>
      <RD.Portal>
        <RD.Overlay className="fixed inset-0 z-40 bg-[var(--scrim)] data-[state=closed]:animate-[ios-fade-out_var(--motion-base)_var(--ease)] data-[state=open]:animate-[ios-fade-in_var(--motion-base)_var(--ease)]" />
        <SheetContent onOpenChange={onOpenChange} dismissible={dismissible} block={block} {...body} />
      </RD.Portal>
    </RD.Root>
  );
}

// Mounted per open, so the detent state starts from the prop each time.
function SheetContent({
  title,
  icon,
  description,
  strongDescription,
  children,
  footer,
  detent: initial = "medium",
  onDetentChange,
  closeLabel,
  handleLabel,
  dismissible,
  closeButton = true,
  tall,
  block,
  onOpenChange,
}: Omit<SheetProps, "open" | "dismissible"> & { dismissible: boolean; block: (e: Event) => void }) {
  const [detent, setDetent] = useState<SheetDetent>(initial);
  const [overflows, setOverflows] = useState(false);
  const downY = useRef<number | null>(null);
  const swiped = useRef(false);
  const content = useRef<HTMLDivElement>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const large = detent === "large";

  const change = (next: SheetDetent) => {
    setDetent(next);
    onDetentChange?.(next);
  };

  // The scroll region keeps title and body; only the handle and footer are
  // pinned. When the content does not fit medium (big text, long body) the
  // sheet opens at large once, and the region is a keyboard stop only while it scrolls.
  const opened = useRef(false);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const measure = () => {
      const over = el.scrollHeight > el.clientHeight + 1;
      setOverflows(over);
      if (over && !opened.current && detent === "medium") change("large");
      opened.current = true;
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    // The box and what is in it: content grows with big text or a late font.
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    for (const child of Array.from(el.children)) ro.observe(child);
    void document.fonts?.ready.then(measure);
    return () => ro.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const release = () => {
    downY.current = null;
  };

  return (
    <RD.Content
      ref={content}
      // Focus lands on the sheet itself, not the drag handle (no focus ring on open).
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        content.current?.focus();
      }}
      onEscapeKeyDown={block}
      onPointerDownOutside={block}
      onInteractOutside={block}
      // No description: tell Radix so (an explicit undefined), rather than
      // repeating the title as one.
      {...(description ? {} : { "aria-describedby": undefined })}
      data-detent={detent}
      className={cn(
        "fixed inset-x-0 bottom-0 z-50 flex flex-col rounded-t-(--ios-radius-sheet) border border-b-0 border-line2 bg-surface text-ink shadow-float outline-none",
        "data-[state=closed]:animate-[ios-sheet-out_var(--motion-base)_var(--ease)] data-[state=open]:animate-[ios-sheet-in_var(--motion-slow)_var(--ease)]",
        large ? "h-[calc(100dvh-var(--safe-top)-0.75rem)]" : tall ? "max-h-[calc(62dvh+var(--safe-bottom,0px))]" : "max-h-[calc(55dvh+var(--safe-bottom,0px))]",
      )}
    >
      <div className="relative shrink-0">
        <button
          type="button"
          aria-label={handleLabel}
          aria-expanded={large}
          onPointerDown={(e) => {
            downY.current = e.clientY;
            swiped.current = false;
            // Keep the gesture if the finger leaves the handle.
            try {
              e.currentTarget.setPointerCapture(e.pointerId);
            } catch {
              /* no capture (tests, old engines): the gesture ends on pointerup/cancel anyway */
            }
          }}
          onPointerUp={(e) => {
            if (downY.current === null) return;
            const dy = e.clientY - downY.current;
            release();
            if (Math.abs(dy) < SWIPE) return;
            swiped.current = true;
            if (dy < 0) change("large");
            else if (large) change("medium");
            else if (dismissible) onOpenChange(false);
          }}
          onPointerCancel={release}
          onLostPointerCapture={release}
          onClick={() => {
            if (swiped.current) swiped.current = false;
            else change(large ? "medium" : "large");
          }}
          className="mx-auto grid min-h-ios-target w-full touch-none place-items-center"
        >
          <span aria-hidden="true" className="h-[5px] w-9 rounded-[3px] bg-line2" />
        </button>
        {dismissible && closeButton && (
          <RD.Close aria-label={closeLabel} className="absolute top-0 right-1 grid min-h-ios-target min-w-ios-target place-items-center rounded-full text-muted">
            <Icon name="close" size={22} />
          </RD.Close>
        )}
      </div>
      <div ref={scroller} tabIndex={overflows ? 0 : undefined} className="min-h-0 flex-1 overflow-y-auto px-5 pb-2">
        {icon && <Icon name={icon} size={32} className="mb-2 size-8 text-muted" />}
        <RD.Title className="text-ios-title3 m-0 pe-8">{title}</RD.Title>
        {description && <RD.Description className={cn("text-ios-callout m-0 mt-2", strongDescription ? "text-ink" : "text-muted")}>{description}</RD.Description>}
        <div className="mt-2.5">{children}</div>
      </div>
      {footer && (
        <div data-sheet-footer className="flex shrink-0 flex-col gap-1 px-5 pt-3 pb-safe">
          {footer}
        </div>
      )}
      {!footer && <div className="pb-safe" />}
    </RD.Content>
  );
}
