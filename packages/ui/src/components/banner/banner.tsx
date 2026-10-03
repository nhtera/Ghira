// SPDX-License-Identifier: Apache-2.0
// Inline banner for conditions the user should know about while they use the
// screen: pocket (muffled sound), too hot, record-only device. Icon + text, the
// tint only reinforces. Warning is amber, info is the accent.
import type { ReactNode } from "react";
import { Icon, type IconName } from "../../icons/icon";
import { cn } from "../../utils/cn";

export type BannerVariant = "warning" | "info";

type Dismiss = { onDismiss?: undefined; dismissLabel?: undefined } | { onDismiss: () => void; dismissLabel: string };

export type BannerProps = Dismiss & {
  variant: BannerVariant;
  /** Defaults to the variant's glyph. */
  icon?: IconName;
  title: string;
  /** Detail under the title. */
  children?: ReactNode;
  action?: { label: string; onPress: () => void };
  className?: string;
};

const LOOK: Record<BannerVariant, { icon: IconName; tone: string; mark: string }> = {
  warning: { icon: "warning", tone: "bg-warn-soft", mark: "text-warn" },
  info: { icon: "info", tone: "bg-accent-soft", mark: "text-accent" },
};

export function Banner({ variant, icon, title, children, action, onDismiss, dismissLabel, className }: BannerProps) {
  const look = LOOK[variant];
  return (
    <div role="status" data-variant={variant} className={cn("flex items-start gap-3 rounded-(--ios-radius-group) py-3 ps-3.5 pe-1 text-ink", look.tone, className)}>
      <Icon name={icon ?? look.icon} size={22} className={cn("mt-0.5 size-[1.375rem] shrink-0", look.mark)} />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5 py-0.5">
        <p className="text-ios-subhead m-0 font-semibold">{title}</p>
        {children && <p className="text-ios-footnote m-0">{children}</p>}
        {action && (
          <button type="button" onClick={action.onPress} className={cn("text-ios-subhead -ms-1 mt-1 min-h-ios-target self-start px-1 text-start font-semibold underline underline-offset-2", look.mark)}>
            {action.label}
          </button>
        )}
      </div>
      {onDismiss && (
        <button type="button" onClick={onDismiss} aria-label={dismissLabel} className="-mt-1 grid min-h-ios-target min-w-ios-target shrink-0 place-items-center text-muted">
          <Icon name="close" size={20} />
        </button>
      )}
    </div>
  );
}
