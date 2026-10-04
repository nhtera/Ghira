// SPDX-License-Identifier: Apache-2.0
// One onboarding step: the heading and body scroll, the actions stay at the
// bottom inside the safe area (thumb reach), at any text size. The icon is
// the design's plain accent glyph above the title.
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
      <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-4">
        {icon && (
          <Icon name={icon} size={44} className="mt-1 mb-4 size-11 text-accent" />
        )}
        <h1
          tabIndex={-1}
          data-step-title
          className="text-ios-title1 m-0 outline-none"
        >
          {title}
        </h1>
        {subtitle && (
          <p className="text-ios-body m-0 mt-3 text-muted">{subtitle}</p>
        )}
        {children && <div className="mt-6">{children}</div>}
      </div>
      <div className="flex flex-col gap-2 px-6 pt-3 pb-[calc(var(--safe-bottom,0px)+1rem)]">{footer}</div>
    </div>
  );
}
