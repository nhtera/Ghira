// SPDX-License-Identifier: Apache-2.0
// Consent to save someone's voice (design note #2): a separate dialog, no
// default choice, Save stays off until one of the two answers is picked.
// Escape and Cancel save nothing.
import { formatDate, APP_NAME, type Locale } from "@ghi/i18n";
import { useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { Avatar, Button, Dialog, Icon, cn, type IconName } from "@ghi/ui";

type Answer = "yes" | "notYet";

const POINTS: Array<{ key: "biometric" | "local" | "deletable"; icon: IconName }> = [
  { key: "biometric", icon: "fingerprint" },
  { key: "local", icon: "lock" },
  { key: "deletable", icon: "delete" },
];

export type ConsentDialogProps = {
  open: boolean;
  name: string;
  /** `agreed`: they said yes (a voice profile may be saved); else the name only. */
  onConfirm: (agreed: boolean) => void;
  /** Escape, Cancel: nothing is saved. */
  onCancel: () => void;
};

export function ConsentDialog({ open, name, onConfirm, onCancel }: ConsentDialogProps) {
  const { i18n } = useTranslation();
  // Remount per opening: a previous answer must never be pre-selected.
  return open ? <Body key={name} name={name} locale={i18n.language as Locale} onConfirm={onConfirm} onCancel={onCancel} /> : null;
}

function Body({ name, locale, onConfirm, onCancel }: { name: string; locale: Locale; onConfirm: (agreed: boolean) => void; onCancel: () => void }) {
  const { t } = useTranslation();
  const [answer, setAnswer] = useState<Answer | null>(null);
  // The date the consent is recorded: when the dialog opened.
  const [openedAt] = useState(() => Date.now());
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      onCancel();
    }
  };
  const options: Array<{ value: Answer; label: string }> = [
    { value: "yes", label: t("consent.yes", { name }) },
    { value: "notYet", label: t("consent.notYet") },
  ];
  return (
    <Dialog
      open
      onOpenChange={() => {}}
      dismissible={false}
      width={540}
      title={
        <span className="flex items-center gap-3">
          <Avatar kind="person" name={name} colorSlot={4} size="xl" />
          {t("consent.title", { name })}
        </span>
      }
      description={t("consent.lead", { app: APP_NAME, name })}
      footer={
        <>
          <Button onClick={onCancel}>{t("common.cancel")}</Button>
          <Button variant="primary" disabled={answer === null} onClick={() => answer && onConfirm(answer === "yes")}>
            {answer === "notYet" ? t("consent.saveNameOnly") : t("consent.save")}
          </Button>
        </>
      }
    >
      <div onKeyDown={onKeyDown} className="flex flex-col gap-3">
        <ul className="m-0 flex list-none flex-col gap-2.5 rounded-row bg-surface2 px-4 py-3.5 text-[13.5px] leading-normal">
          {POINTS.map((p) => (
            <li key={p.key} className="flex gap-2.5">
              <Icon name={p.icon} size={19} className="flex-none text-accent" />
              {t(`consent.points.${p.key}`, { name })}
            </li>
          ))}
        </ul>
        <fieldset className="m-0 flex flex-col gap-1.5 border-0 p-0">
          <legend className="mb-1 p-0 text-[13.5px] font-semibold">{t("consent.ask", { name })}</legend>
          {options.map((o) => (
            <label
              key={o.value}
              className={cn(
                "flex min-h-11 cursor-pointer items-center gap-2.5 rounded-row border-[1.5px] px-3 py-2 text-[13.5px] has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-accent",
                answer === o.value ? "border-accent bg-accent-soft" : "border-ctl bg-surface hover:bg-surface2",
              )}
            >
              <input type="radio" name="consent-answer" value={o.value} checked={answer === o.value} onChange={() => setAnswer(o.value)} className="sr-only" />
              <Icon name={answer === o.value ? "radio_button_checked" : "radio_button_unchecked"} size={20} className={answer === o.value ? "text-accent" : "text-muted"} />
              <span>{o.label}</span>
            </label>
          ))}
        </fieldset>
        <p role="status" className="m-0 min-h-4 text-[12px] text-muted">
          {answer === null ? t("consent.pick") : answer === "yes" ? t("consent.record", { app: APP_NAME, name, date: formatDate(openedAt, locale) }) : ""}
        </p>
      </div>
    </Dialog>
  );
}
