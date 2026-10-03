// SPDX-License-Identifier: Apache-2.0
// One onboarding step: the heading and body scroll, the actions stay at the
// bottom inside the safe area (thumb reach), at any text size.
import { Icon, type IconName } from "@ghi/ui";
import type { ReactNode } from "react";

export type StepLayoutProps = {
  icon?: IconName;
  title: string;
  subtitle?: string;
  children?: ReactNode;
  /** The actions, primary first. */
  footer: ReactNode;
};

export function StepLayout({
  icon,
  title,
  subtitle,
  children,
  footer,
}: StepLayoutProps) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto px-5 pb-4">
        {icon && (
          <span
            aria-hidden="true"
            className="mt-2 mb-4 grid size-14 place-items-center rounded-full bg-accent-soft text-accent"
          >
            <Icon name={icon} size={30} className="size-[1.875rem]" />
          </span>
        )}
        <h1
          tabIndex={-1}
          data-step-title
          className="text-ios-title1 m-0 outline-none"
        >
          {title}
        </h1>
        {subtitle && (
          <p className="text-ios-body m-0 mt-2 text-muted">{subtitle}</p>
        )}
        {children && <div className="mt-5">{children}</div>}
      </div>
      <div className="flex flex-col gap-2 px-5 pt-3 pb-safe">{footer}</div>
    </div>
  );
}
