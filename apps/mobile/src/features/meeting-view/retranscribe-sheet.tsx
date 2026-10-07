// SPDX-License-Identifier: Apache-2.0
// "Transcribe again" (Transcript tab): the final pass runs again on the audio
// kept on this phone, in the spoken language chosen here (the paired computer
// may take it, as after any recording). Names and edited lines stay.
import { Icon, PhoneButton, Sheet, cn } from "@ghi/ui";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingDetail, TranscriptLanguage } from "../../bindings";
import { ipc } from "../../ipc";

/** What the meeting was transcribed in, as the sheet's first choice. */
export function initialLanguage(detail: Pick<MeetingDetail, "language">): TranscriptLanguage {
  return detail.language === "en" || detail.language === "vi" ? detail.language : "auto";
}

export type RetranscribeSheetProps = {
  open: boolean;
  detail: MeetingDetail;
  onClose: () => void;
  /** After the core took it: `waiting` when the speech models aren't installed yet. */
  onStarted: (waiting: boolean) => void;
};

export function RetranscribeSheet({ open, detail, onClose, onStarted }: RetranscribeSheetProps) {
  const { t } = useTranslation();
  const [language, setLanguage] = useState<TranscriptLanguage>(() => initialLanguage(detail));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const options: { value: TranscriptLanguage; label: string }[] = [
    { value: "auto", label: t("mobile.detail.retranscribe.auto") },
    { value: "en", label: t("onboarding.languages.english") },
    { value: "vi", label: t("onboarding.languages.vietnamese") },
  ];

  const close = () => {
    setError(null);
    onClose();
  };
  const confirm = async () => {
    setBusy(true);
    setError(null);
    const r = await ipc.commands.retranscribe(detail.gid, language).catch((e: unknown) => ({ status: "error" as const, error: String(e) }));
    setBusy(false);
    if (r.status === "error") return setError(r.error);
    onStarted(r.data);
    onClose();
  };

  return (
    <Sheet
      open={open}
      onOpenChange={(o) => !o && close()}
      title={t("mobile.detail.retranscribe.action")}
      closeLabel={t("mobile.sheet.close")}
      handleLabel={t("mobile.sheet.handle")}
    >
      <p className="text-ios-subhead m-0 mb-3 text-muted">{t("mobile.detail.retranscribe.body")}</p>
      <div role="radiogroup" aria-label={t("mobile.detail.retranscribe.language")} className="mb-4 flex flex-col overflow-hidden rounded-(--ios-radius-group) bg-surface2">
        {options.map((o, i) => (
          <button
            key={o.value}
            type="button"
            role="radio"
            aria-checked={language === o.value}
            onClick={() => setLanguage(o.value)}
            className={cn("text-ios-body min-h-ios-target flex items-center justify-between px-4 text-left", i > 0 && "border-t border-line")}
          >
            {o.label}
            {language === o.value && <Icon name="check" size={20} className="text-accent" />}
          </button>
        ))}
      </div>
      {error && (
        <p role="alert" className="text-ios-footnote m-0 mb-3 text-warn">
          {t("system.commandFailed", { message: error })}
        </p>
      )}
      <PhoneButton variant="primary" icon="subtitles" disabled={busy} onClick={() => void confirm()}>
        {t("mobile.detail.retranscribe.confirm")}
      </PhoneButton>
    </Sheet>
  );
}
