// SPDX-License-Identifier: Apache-2.0
// What every step shares: the heading block and the action row. The primary
// button carries `data-onboarding-primary` so Enter can press it.
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button, Icon, cn, type ButtonProps } from "@ghi/ui";

/** The design's onboarding button: 42 px tall, 10 px corners, 14 px text (the `!` beats the Button size). */
export const OB_BUTTON = "h-[42px]! rounded-[10px]! px-5.5! text-[14px]!";

export type StepNav = {
  next: () => void;
  back: () => void;
  /** Mark onboarding done and leave. */
  finish: () => void;
  canGoBack: boolean;
};

export function StepFrame({ title, body, serif, children }: { title: ReactNode; body?: ReactNode; serif?: boolean; children?: ReactNode }) {
  return (
    <div className="flex max-w-[600px] flex-col gap-4.5">
      <h1 data-onboarding-title tabIndex={-1} className={serif ? "text-display m-0 outline-none" : "text-title m-0 outline-none"}>
        {title}
      </h1>
      {body && <p className={`m-0 leading-relaxed text-muted ${serif ? "text-[15px]" : "text-[14px]"}`}>{body}</p>}
      {children}
    </div>
  );
}

/** Back, the primary action, and an optional skip: nothing in onboarding blocks. */
export function StepActions({
  nav,
  primary,
  onPrimary,
  primaryProps,
  skip,
  onSkip,
  start,
  enter = true,
}: {
  nav: StepNav;
  primary?: string;
  onPrimary?: () => void;
  primaryProps?: Partial<ButtonProps>;
  /** A quiet "skip" next to the primary action. */
  skip?: string;
  onSkip?: () => void;
  /** Extra controls before the primary action. */
  start?: ReactNode;
  /** Enter presses the primary button (off where another control should take Enter). */
  enter?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <>
      <div className="mt-2 flex flex-wrap items-center gap-2">
        {start}
        <Button variant="primary" size="lg" data-onboarding-primary={enter ? "" : undefined} onClick={onPrimary ?? nav.next} {...primaryProps} className={cn(OB_BUTTON, primaryProps?.className)}>
          {primary ?? t("common.continue")}
        </Button>
        {skip && (
          <Button variant="ghost" size="lg" className={cn(OB_BUTTON, "px-3.5! text-[13.5px]! font-medium text-muted")} onClick={onSkip ?? nav.next}>
            {skip}
          </Button>
        )}
      </div>
      {nav.canGoBack && (
        <div>
          <Button variant="ghost" size="sm" className="-ml-2 text-[13px]! font-normal text-muted" icon="chevron_left" onClick={nav.back}>
            {t("common.back")}
          </Button>
        </div>
      )}
    </>
  );
}

/** A status line with an icon, so state never relies on color alone. */
export function Notice({ tone = "info", icon, children }: { tone?: "info" | "warn"; icon?: Parameters<typeof Icon>[0]["name"]; children: ReactNode }) {
  return (
    <div role="status" className={`flex items-start gap-2 rounded-ctl px-3 py-2.5 text-[12.5px] leading-snug ${tone === "warn" ? "bg-warn-soft text-warn" : "bg-surface2 text-muted"}`}>
      {icon && <Icon name={icon} size={16} className="mt-px flex-none" />}
      <span className="min-w-0">{children}</span>
    </div>
  );
}
