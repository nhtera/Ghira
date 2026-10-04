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

export const PhoneButton = forwardRef<HTMLButtonElement, PhoneButtonProps>(
  function PhoneButton(
    {
      variant = "primary",
      icon,
      inline,
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
          "text-ios-headline inline-flex min-h-ios-button items-center justify-center gap-2.5 rounded-(--ios-radius-group) py-3 text-center font-semibold",
          inline ? "min-w-40 px-6" : "w-full px-5",
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
