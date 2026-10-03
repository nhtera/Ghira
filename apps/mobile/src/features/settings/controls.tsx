// SPDX-License-Identifier: Apache-2.0
// Phone-sized controls the settings screens share: a switch, buttons with 44 pt
// targets, a choice row (radio-like) and the error line.
import { cn, Icon, ListRow } from "@ghi/ui";
import type {
  ComponentPropsWithRef,
  InputHTMLAttributes,
  ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import { knownError } from "./api";

export function Switch({
  checked,
  onChange,
  labelledBy,
  disabled,
}: {
  checked: boolean;
  onChange: (on: boolean) => void;
  labelledBy: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-labelledby={labelledBy}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className="grid min-h-ios-target min-w-ios-target place-items-center disabled:opacity-50"
    >
      <span
        aria-hidden="true"
        className={cn(
          "relative block h-[1.9375rem] w-[3.1875rem] rounded-full transition-colors duration-(--motion-fast) motion-reduce:transition-none",
          checked ? "bg-accent" : "bg-ctl",
        )}
      >
        <span
          className={cn(
            "absolute top-0.5 block size-[1.6875rem] rounded-full bg-on-accent shadow transition-transform duration-(--motion-fast) motion-reduce:transition-none",
            checked ? "translate-x-[1.3125rem]" : "translate-x-0.5",
          )}
        />
      </span>
    </button>
  );
}

type Tone = "primary" | "secondary" | "danger";
const TONE: Record<Tone, string> = {
  primary: "bg-accent text-on-accent",
  secondary: "border border-ctl bg-surface text-ink",
  danger: "bg-rec-soft text-rec-ink",
};

export function Btn({
  tone = "secondary",
  className,
  ...rest
}: ComponentPropsWithRef<"button"> & { tone?: Tone }) {
  return (
    <button
      type="button"
      className={cn(
        "text-ios-body min-h-ios-target rounded-(--ios-radius-group) px-4 py-2 font-semibold disabled:opacity-50",
        TONE[tone],
        className,
      )}
      {...rest}
    />
  );
}

/** One option of a pick-one group: a row that says "Selected" to assistive tech. */
export function ChoiceRow({
  title,
  subtitle,
  selected,
  onPress,
  disabled,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  selected: boolean;
  onPress: () => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <ListRow
      title={title}
      subtitle={subtitle}
      disabled={disabled}
      onPress={onPress}
      value={
        selected ? (
          <span className="inline-flex items-center text-accent">
            <Icon name="check" size={20} />
            <span className="sr-only">{t("mobile.settings.selected")}</span>
          </span>
        ) : undefined
      }
    />
  );
}

/** An error code as a sentence (role=alert). */
export function ErrorLine({
  code,
  fallback,
}: {
  code: string | null;
  fallback?: "mobile.settings.saveFailed";
}) {
  const { t } = useTranslation();
  if (!code) return null;
  const k = knownError(code);
  return (
    <p role="alert" className="text-ios-footnote mx-4 my-2 text-rec-ink">
      {k === "generic" && fallback
        ? t(fallback)
        : t(`mobile.settings.error.${k}`)}
    </p>
  );
}

/** A text field styled like the lists: label above, 44 pt tall. */
export function Field({
  id,
  label,
  className,
  ...input
}: { id: string; label: string } & Omit<
  InputHTMLAttributes<HTMLInputElement>,
  "id"
>) {
  return (
    <div className="flex flex-col gap-1">
      <label
        htmlFor={id}
        className="text-ios-footnote px-1 font-medium text-muted"
      >
        {label}
      </label>
      <input
        id={id}
        className={cn(
          "text-ios-body min-h-ios-target rounded-(--ios-radius-group) border border-ctl bg-surface px-3 text-ink select-text",
          className,
        )}
        {...input}
      />
    </div>
  );
}
