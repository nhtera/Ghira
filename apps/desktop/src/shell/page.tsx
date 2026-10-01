// SPDX-License-Identifier: Apache-2.0
// A screen's frame: header (a window drag region, since the mac title bar is
// transparent) and a scrolling body.
import type { ReactNode } from "react";
import { cn } from "@ghi/ui";

export function Page({ title, subtitle, actions, children, className }: { title: ReactNode; subtitle?: ReactNode; actions?: ReactNode; children?: ReactNode; className?: string }) {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <header data-tauri-drag-region className="flex flex-none items-end gap-3 px-7 pt-6 pb-4">
        <div data-tauri-drag-region className="min-w-0 flex-1">
          <h1 className="text-title m-0 truncate">{title}</h1>
          {subtitle && <p className="text-small m-0 mt-0.5 text-muted">{subtitle}</p>}
        </div>
        {actions}
      </header>
      <div className={cn("min-h-0 flex-1 overflow-auto px-7 pb-7", className)}>{children}</div>
    </div>
  );
}
