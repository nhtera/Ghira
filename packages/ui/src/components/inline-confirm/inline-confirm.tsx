// SPDX-License-Identifier: Apache-2.0
// Destructive confirmation without a browser/system dialog: the panel takes
// the trigger's place, focus moves to Cancel (the safe default), Escape or
// Cancel puts focus back on the trigger.
import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode, type Ref } from "react";
import { useTranslation } from "react-i18next";
import { Icon, type IconName } from "../../icons/icon";
import { Button } from "../../primitives/button";
import { cn } from "../../utils/cn";

export type InlineConfirmProps = {
  /** The consequence, in full sentences. */
  question: ReactNode;
  confirmLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
  icon?: IconName;
  /** `warn`: for a change that is not a data loss (removing a name from notes). */
  tone?: "danger" | "warn";
  className?: string;
};

export function InlineConfirm({ question, confirmLabel, onConfirm, onCancel, icon = "delete_forever", tone = "danger", className }: InlineConfirmProps) {
  const { t } = useTranslation();
  const id = useId();
  const warn = tone === "warn";
  const cancelRef = useRef<HTMLButtonElement>(null);
  useEffect(() => cancelRef.current?.focus(), []);
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      onCancel();
    }
  };
  return (
    <div
      role="alertdialog"
      aria-labelledby={id}
      onKeyDown={onKeyDown}
      className={cn("flex flex-wrap items-center gap-2.5 rounded-row border-[1.5px] px-3.5 py-3", warn ? "border-warn bg-warn-soft" : "border-rec bg-rec-soft", className)}
    >
      <Icon name={icon} size={20} className={warn ? "text-warn" : "text-rec-ink"} />
      <b id={id} className={cn("text-body min-w-48 flex-1 font-semibold", warn ? "text-warn" : "text-rec-ink")}>
        {question}
      </b>
      <Button variant="danger" className={warn ? "bg-warn text-surface" : undefined} onClick={onConfirm}>
        {confirmLabel}
      </Button>
      <Button ref={cancelRef} onClick={onCancel}>
        {t("common.cancel")}
      </Button>
    </div>
  );
}

export type ConfirmAreaProps = Omit<InlineConfirmProps, "onCancel" | "onConfirm"> & {
  /** Renders the trigger; call `onClick` to ask. Attach `ref` so focus can return. */
  trigger: (p: { onClick: () => void; ref: Ref<HTMLButtonElement> }) => ReactNode;
  onConfirm: () => void;
  onCancel?: () => void;
};

/** Trigger that swaps itself for an InlineConfirm and back. */
export function ConfirmArea({ trigger, onConfirm, onCancel, ...panel }: ConfirmAreaProps) {
  const [asking, setAsking] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef(false);
  useEffect(() => {
    if (!asking && returnFocus.current) {
      returnFocus.current = false;
      triggerRef.current?.focus();
    }
  }, [asking]);
  if (asking) {
    return (
      <InlineConfirm
        {...panel}
        onConfirm={() => {
          setAsking(false);
          onConfirm();
        }}
        onCancel={() => {
          returnFocus.current = true;
          setAsking(false);
          onCancel?.();
        }}
      />
    );
  }
  return <>{trigger({ onClick: () => setAsking(true), ref: triggerRef })}</>;
}
