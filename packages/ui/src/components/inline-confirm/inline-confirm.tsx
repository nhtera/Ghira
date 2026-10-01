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
  className?: string;
};

export function InlineConfirm({ question, confirmLabel, onConfirm, onCancel, icon = "delete_forever", className }: InlineConfirmProps) {
  const { t } = useTranslation();
  const id = useId();
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
      className={cn("flex flex-wrap items-center gap-2.5 rounded-row border-[1.5px] border-rec bg-rec-soft px-3.5 py-3", className)}
    >
      <Icon name={icon} size={20} className="text-rec-ink" />
      <b id={id} className="text-body min-w-48 flex-1 font-semibold text-rec-ink">
        {question}
      </b>
      <Button variant="danger" onClick={onConfirm}>
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
