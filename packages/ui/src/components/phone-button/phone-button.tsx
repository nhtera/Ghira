// SPDX-License-Identifier: Apache-2.0
// The phone app's text button (full width by default) at phone size: 52 pt tall at least (the design; 44 pt is the hit-target floor), the label wraps
// instead of clipping (Vietnamese is long, text goes to 200%). The desktop
// Button is a 36 px control that never wraps.
import { Icon, type IconName } from "../../icons/icon";
import { cn } from "../../utils/cn";
import { forwardRef, type ButtonHTMLAttributes } from "react";

export type PhoneButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "secondary" | "ghost" | "danger";
  icon?: IconName;
  /** regular: 52 pt, 18 px. small: 52 pt, 16 px (the design's own button text; long labels). compact: 44 pt, 17 px (Cancel, Stop and save). */
  size?: "regular" | "small" | "compact";
  /** Hug the label instead of filling the row. */
  inline?: boolean;
};

const VARIANT = {
  primary: "bg-accent text-on-accent active:brightness-90",
  secondary: "border border-ctl bg-surface text-ink active:bg-sunk",
  ghost: "bg-transparent text-accent active:bg-sunk",
  /** Destructive text action (Stop and save): red text, no fill. */
  danger: "bg-transparent text-rec-ink active:bg-sunk",
};

const SIZE = {
  regular: "text-ios-headline min-h-ios-button py-3",
  small: "text-ios-subhead min-h-ios-button py-3",
  compact: "text-ios-callout min-h-ios-target py-1.5",
};

export const PhoneButton = forwardRef<HTMLButtonElement, PhoneButtonProps>(
  function PhoneButton(
    {
      variant = "primary",
      icon,
      inline,
      size = "regular",
      className,
      children,
      type = "button",
      ...rest
    },
    ref,
  ) {
    return (
      <button
        ref={ref}
        type={type}
        className={cn(
          "inline-flex items-center justify-center gap-2.5 rounded-(--ios-radius-group) text-center font-semibold",
          SIZE[size],
          inline ? "min-w-40 px-6" : "w-full px-4",
          "disabled:cursor-not-allowed disabled:opacity-50",
          VARIANT[variant],
          className,
        )}
        {...rest}
      >
        {icon && <Icon name={icon} size={24} className="size-6 shrink-0" />}
        {children}
      </button>
    );
  },
);
