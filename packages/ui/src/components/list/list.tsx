// SPDX-License-Identifier: Apache-2.0
// Inset grouped list (iOS Settings style): a rounded group of rows with
// hairline separators inset from the leading edge, an optional header above
// and footer below.
import { useId, type ReactNode } from "react";
import { Icon, type IconName } from "../../icons/icon";
import { cn } from "../../utils/cn";

export type ListSectionProps = {
  header?: string;
  footer?: string;
  children: ReactNode;
  className?: string;
};

export function ListSection({ header, footer, children, className }: ListSectionProps) {
  const id = useId();
  return (
    <section className={cn("mx-4 my-3", className)}>
      {header && (
        <h2 id={id} className="text-ios-footnote m-0 px-4 pb-1.5 font-medium text-muted">
          {header}
        </h2>
      )}
      <ul aria-labelledby={header ? id : undefined} className="m-0 list-none overflow-hidden rounded-(--ios-radius-group) bg-surface p-0">
        {children}
      </ul>
      {footer && <p className="text-ios-footnote m-0 px-4 pt-1.5 text-muted">{footer}</p>}
    </section>
  );
}

type RowBase = {
  title: ReactNode;
  subtitle?: ReactNode;
  icon?: IconName;
  /** Secondary text at the trailing edge. */
  value?: ReactNode;
  destructive?: boolean;
  disabled?: boolean;
  className?: string;
};

// A row is a button (onPress) or holds its own control (trailing), never both:
// a control nested in a button is not operable by assistive tech.
export type ListRowProps = RowBase &
  (
    | {
        /** Makes the row a button. */
        onPress: () => void;
        /** Disclosure chevron: set it on rows that navigate. */
        chevron?: boolean;
        /** For a row that expands content below it: its state and the id of that content. */
        expanded?: boolean;
        controls?: string;
        trailing?: never;
      }
    | {
        onPress?: undefined;
        chevron?: never;
        expanded?: never;
        controls?: never;
        /**
         * A control at the trailing edge (a switch). Pass a function to get the
         * id of the title, for the control's `aria-labelledby`.
         */
        trailing?: ReactNode | ((titleId: string) => ReactNode);
      }
  );

export function ListRow({ title, subtitle, icon, value, trailing, chevron, expanded, controls, onPress, destructive, disabled, className }: ListRowProps) {
  const titleId = useId();
  const inner = (
    <>
      {icon && (
        <span aria-hidden="true" className="grid size-[1.875rem] shrink-0 place-items-center rounded-[0.4375rem] bg-accent-soft text-accent">
          <Icon name={icon} size={18} className="size-[1.125rem]" />
        </span>
      )}
      <span className="flex min-w-[5rem] flex-1 flex-col text-start">
        <span id={titleId} className={cn("text-ios-body", destructive && "text-rec-ink", disabled && "text-muted")}>
          {title}
        </span>
        {subtitle && <span className="text-ios-footnote text-muted">{subtitle}</span>}
      </span>
      {value !== undefined && <span className="text-ios-body ms-auto text-end text-muted">{value}</span>}
      {typeof trailing === "function" ? trailing(titleId) : trailing}
      {chevron && <Icon name="chevron_right" size={20} className="size-5 shrink-0 text-muted" />}
    </>
  );
  // Wraps: at large text the value drops under the title instead of squeezing it.
  const row = "flex min-h-ios-target w-full flex-wrap items-center gap-x-3 gap-y-0.5 px-4 py-2";
  return (
    <li className={cn("relative after:absolute after:right-0 after:bottom-0 after:left-4 after:h-px after:bg-line last:after:hidden", className)}>
      {onPress ? (
        <button type="button" onClick={onPress} disabled={disabled} aria-expanded={expanded} aria-controls={expanded ? controls : undefined} className={cn(row, "active:bg-sunk")}>
          {inner}
        </button>
      ) : (
        <div className={row}>{inner}</div>
      )}
    </li>
  );
}
