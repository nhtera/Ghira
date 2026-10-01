// SPDX-License-Identifier: Apache-2.0
// Buttons: primary (teal), secondary (outlined, `ctl` border), ghost, danger
// (record red: destructive only). Icon-only buttons need `aria-label`.
import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import { Icon, type IconName } from "../icons/icon";
import { cn } from "../utils/cn";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md" | "lg";

const VARIANT: Record<ButtonVariant, string> = {
  primary: "bg-accent text-on-accent hover:brightness-110",
  secondary: "bg-surface text-ink border border-ctl hover:bg-surface2",
  ghost: "bg-transparent text-ink hover:bg-sunk",
  danger: "bg-rec text-on-accent hover:brightness-110",
};
const SIZE: Record<ButtonSize, string> = {
  sm: "h-7 px-2.5 gap-1.5 text-[12.5px]",
  md: "h-8 px-3 gap-1.5 text-[13px]",
  lg: "h-9 px-3.5 gap-2 text-[13px]",
};
const ICON_ONLY: Record<ButtonSize, string> = { sm: "size-7", md: "size-8", lg: "size-9" };

export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: IconName;
  /** Trailing content (a shortcut hint, a chevron). */
  end?: ReactNode;
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "secondary", size = "md", icon, end, className, children, type = "button", ...rest },
  ref,
) {
  const iconOnly = icon && !children;
  return (
    <button
      ref={ref}
      type={type}
      className={cn(
        "inline-flex shrink-0 items-center justify-center rounded-ctl font-semibold whitespace-nowrap",
        "transition-[filter,background-color] duration-(--motion-fast) ease-out",
        "disabled:cursor-not-allowed disabled:opacity-50",
        VARIANT[variant],
        iconOnly ? ICON_ONLY[size] : SIZE[size],
        className,
      )}
      {...rest}
    >
      {icon && <Icon name={icon} size={size === "sm" ? 15 : 17} />}
      {children}
      {end}
    </button>
  );
});
