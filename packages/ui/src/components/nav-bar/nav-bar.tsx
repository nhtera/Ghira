// SPDX-License-Identifier: Apache-2.0
// Navigation bar with a large title that collapses into the bar as the page
// scrolls (iOS). The large title is the first child of the scroll content
// (`LargeTitle`, the page's h1); `useLargeTitleCollapse` watches it with an
// IntersectionObserver, so there is no scroll handler and no jump. The inline
// copy in the bar is decorative while a large title exists.
import { forwardRef, useEffect, useRef, useState, type ReactNode } from "react";
import { Icon } from "../../icons/icon";
import { cn } from "../../utils/cn";
import { useMobileT } from "../../utils/mobile-t";

type Back = { onBack?: undefined; backLabel?: undefined } | { onBack: () => void; backLabel?: string };

export type NavBarProps = Back & {
  title: string;
  /** A `LargeTitle` carries the heading in the scroll content. false: this bar holds the h1. */
  large?: boolean;
  /** The large title scrolled out of view (see useLargeTitleCollapse). */
  collapsed?: boolean;
  /** Right side: buttons (44 pt each). */
  trailing?: ReactNode;
  className?: string;
};

/** `collapsed` for a scroll container: put `scrollRef` on it and `titleRef` on its `LargeTitle`. */
export function useLargeTitleCollapse<T extends HTMLElement = HTMLDivElement>(
  /** false while the title is not rendered yet (a screen that loads first); the observer starts when it turns true. */
  ready = true,
) {
  const [collapsed, setCollapsed] = useState(false);
  const scrollRef = useRef<T>(null);
  const titleRef = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    const scroller = scrollRef.current;
    const title = titleRef.current;
    if (!ready || !scroller || !title || typeof IntersectionObserver === "undefined") return;
    // Collapsed once the title has left the scroller (the bar sits above it).
    const io = new IntersectionObserver(([entry]) => setCollapsed(!entry.isIntersecting), { root: scroller, threshold: 0 });
    io.observe(title);
    return () => io.disconnect();
  }, [ready]);
  return { collapsed, scrollRef, titleRef };
}

/** The page heading, first in the scroll content. */
export const LargeTitle = forwardRef<HTMLHeadingElement, { children: ReactNode; className?: string }>(function LargeTitle({ children, className }, ref) {
  return (
    <h1 ref={ref} className={cn("text-ios-large-title m-0 px-4 pt-1 pb-2", className)}>
      {children}
    </h1>
  );
});

export function NavBar({ title, large = true, collapsed = false, onBack, backLabel, trailing, className }: NavBarProps) {
  const t = useMobileT();
  const inline = !large || collapsed;
  return (
    // @container: the back label gives way to the chevron when the bar is narrow
    // for the text scale (20rem = 320 pt at the default size, 640 pt at 200%).
    <header className={cn("@container bg-bg pt-safe", inline && "border-b border-line", className)} data-collapsed={collapsed}>
      <div className="grid min-h-[var(--ios-nav-h)] grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-2 px-2">
        <div className="flex min-w-0 items-center">
          {onBack && (
            <button
              type="button"
              onClick={onBack}
              aria-label={backLabel ? `${t("mobile.nav.back")}, ${backLabel}` : t("mobile.nav.back")}
              className="-ms-1 flex min-h-ios-target min-w-ios-target max-w-full items-center gap-0.5 text-accent"
            >
              <Icon name="chevron_left" size={28} className="size-7 shrink-0" />
              {backLabel && <span className="text-ios-body hidden truncate @min-[20rem]:inline">{backLabel}</span>}
            </button>
          )}
        </div>
        <div
          aria-hidden={large ? true : undefined}
          className={cn("text-ios-headline max-w-[50cqw] min-w-0 truncate text-center transition-opacity duration-(--motion-base)", inline ? "opacity-100" : "opacity-0")}
        >
          {large ? title : <h1 className="text-ios-headline m-0 truncate">{title}</h1>}
        </div>
        <div className="flex min-w-ios-target items-center justify-end">{trailing}</div>
      </div>
    </header>
  );
}
