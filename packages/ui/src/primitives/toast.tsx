// SPDX-License-Identifier: Apache-2.0
// Toasts: info, success, warning, and action (Undo). 2.8 s, or 5 s when there
// is an action (Foundations, motion). `useToast().show(...)` from anywhere
// under <ToastProvider>.
import * as RToast from "@radix-ui/react-toast";
import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from "react";
import { Icon, type IconName } from "../icons/icon";
import { cn } from "../utils/cn";

export type ToastTone = "info" | "success" | "warning";
export type ToastInput = {
  title: string;
  tone?: ToastTone;
  /** An action button (e.g. Undo); `altText` describes it for screen readers. */
  action?: { label: string; altText: string; onAction: () => void };
};
type Item = ToastInput & { id: number };

const ToastContext = createContext<{ show: (t: ToastInput) => void } | null>(null);

export function useToast() {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast needs <ToastProvider>");
  return ctx;
}

const TONE_ICON: Record<ToastTone, IconName | null> = { info: null, success: "check_circle", warning: "warning" };

export function ToastView({ title, tone = "info", action, open = true, onOpenChange }: ToastInput & { open?: boolean; onOpenChange?: (o: boolean) => void }) {
  const icon = TONE_ICON[tone];
  return (
    <RToast.Root
      open={open}
      onOpenChange={onOpenChange}
      duration={action ? 5000 : 2800}
      data-tone={tone}
      className="flex min-h-10 items-center gap-2.5 rounded-row bg-toast-bg py-2 pr-2 pl-3 text-[13px] text-toast-fg shadow-float"
    >
      {icon && <Icon name={icon} size={17} className={tone === "warning" ? "text-[var(--warn-soft)]" : undefined} />}
      <RToast.Title className="flex-1">{title}</RToast.Title>
      {action && (
        <RToast.Action
          altText={action.altText}
          onClick={action.onAction}
          className={cn("h-7 rounded-seg px-2.5 text-[12.5px] font-semibold", "text-toast-fg underline-offset-2 hover:underline")}
        >
          {action.label}
        </RToast.Action>
      )}
    </RToast.Root>
  );
}

export function ToastProvider({ children, label }: { children: ReactNode; /** Region name (localized). */ label: string }) {
  const [items, setItems] = useState<Item[]>([]);
  const show = useCallback((t: ToastInput) => setItems((xs) => [...xs, { ...t, id: Date.now() + Math.random() }]), []);
  const value = useMemo(() => ({ show }), [show]);
  return (
    <ToastContext.Provider value={value}>
      <RToast.Provider label={label} swipeDirection="down">
        {children}
        {items.map((t) => (
          <ToastView key={t.id} {...t} onOpenChange={(o) => !o && setItems((xs) => xs.filter((x) => x.id !== t.id))} />
        ))}
        <RToast.Viewport className="fixed bottom-4 left-1/2 z-[60] flex w-[min(420px,calc(100vw-32px))] -translate-x-1/2 flex-col gap-2 outline-none" />
      </RToast.Provider>
    </ToastContext.Provider>
  );
}

/** Renders toasts in place (gallery, docs) instead of the screen's toast area. */
export function ToastPreview({ children, label }: { children: ReactNode; label: string }) {
  return (
    <RToast.Provider label={label} duration={Infinity}>
      {children}
      <RToast.Viewport className="m-0 flex list-none flex-col gap-2 p-0" />
    </RToast.Provider>
  );
}
