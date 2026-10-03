// SPDX-License-Identifier: Apache-2.0
// A text button (full width by default) at phone size: 44 pt tall at least, the label wraps
// instead of clipping (Vietnamese is long, text goes to 200%). The desktop
// Button is a 36 px control that never wraps.
import { Icon, cn, type IconName } from "@ghi/ui";
import { forwardRef, type ButtonHTMLAttributes } from "react";

export type PhoneButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "secondary" | "ghost";
  icon?: IconName;
  /** Hug the label instead of filling the row. */
  inline?: boolean;
};

const VARIANT = {
  primary: "bg-accent text-on-accent active:brightness-90",
  secondary: "border border-ctl bg-surface text-ink active:bg-sunk",
  ghost: "bg-transparent text-accent active:bg-sunk",
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
          "text-ios-body inline-flex min-h-ios-target items-center justify-center gap-2 rounded-(--ios-radius-group) py-2.5 text-center font-semibold",
          inline ? "min-w-40 px-6" : "w-full px-5",
          "disabled:cursor-not-allowed disabled:opacity-50",
          VARIANT[variant],
          className,
        )}
        {...rest}
      >
        {icon && <Icon name={icon} size={20} className="size-5 shrink-0" />}
        {children}
      </button>
    );
  },
);
