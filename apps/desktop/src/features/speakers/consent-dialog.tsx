// SPDX-License-Identifier: Apache-2.0
// Consent to save someone's voice (design note #2): a separate dialog, no
// default choice, Save stays off until one of the two answers is picked.
// Escape and Cancel save nothing.
import { formatDate, APP_NAME, type Locale } from "@ghi/i18n";
import { useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { Button, Dialog } from "@ghi/ui";

type Answer = "yes" | "notYet";

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
      title={t("consent.title", { name })}
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
        <ul className="text-body m-0 flex list-disc flex-col gap-1.5 pl-5">
          <li>{t("consent.points.biometric")}</li>
          <li>{t("consent.points.local")}</li>
          <li>{t("consent.points.deletable", { name })}</li>
        </ul>
        <fieldset className="m-0 flex flex-col gap-1.5 border-0 p-0">
          <legend className="text-body mb-1 p-0 font-semibold">{t("consent.ask", { name })}</legend>
          {options.map((o) => (
            <label key={o.value} className="text-body flex cursor-pointer items-start gap-2 rounded-ctl border border-line px-3 py-2 has-[:checked]:border-accent has-[:checked]:bg-accent-soft">
              <input type="radio" name="consent-answer" value={o.value} checked={answer === o.value} onChange={() => setAnswer(o.value)} className="mt-1" />
              <span>{o.label}</span>
            </label>
          ))}
        </fieldset>
        <p role="status" className="text-small m-0 text-muted">
          {answer === null ? t("consent.pick") : answer === "yes" ? t("consent.record", { app: APP_NAME, name, date: formatDate(openedAt, locale) }) : ""}
        </p>
      </div>
    </Dialog>
  );
}
