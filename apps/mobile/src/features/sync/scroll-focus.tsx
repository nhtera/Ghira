// SPDX-License-Identifier: Apache-2.0
// A step with nothing to tap in its scrolling area (the camera does the
// work) still has to be reachable when it overflows at large text: this wrapper
// becomes a keyboard stop only while its scrolling parent overflows.
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

export function ScrollFocus({ children, className }: { children: ReactNode; className?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const [overflows, setOverflows] = useState(false);
  useLayoutEffect(() => {
    const el = ref.current;
    const parent = el?.parentElement?.parentElement;
    if (!el || !parent) return;
    const measure = () => setOverflows(parent.scrollHeight > parent.clientHeight + 1);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(parent);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return (
    <div ref={ref} tabIndex={overflows ? 0 : undefined} className={className}>
      {children}
    </div>
  );
}
