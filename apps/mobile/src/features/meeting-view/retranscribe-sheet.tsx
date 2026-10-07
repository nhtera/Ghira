// SPDX-License-Identifier: Apache-2.0
// "Transcribe again" (Transcript tab): the final pass runs again on the audio
// kept on this phone, in the spoken language chosen here (the paired computer
// may take it, as after any recording). Names and edited lines stay.
// Laid out like the other confirm sheets (consent): icon, title, description,
// the choice, then the action and Cancel in the footer. The language choice is
// the import sheet's (Auto / EN / VI).
import { PhoneButton, Sheet } from "@ghi/ui";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingDetail, TranscriptLanguage } from "../../bindings";
import { ipc } from "../../ipc";
import { ChoiceGroup } from "../import-inbox/choice-group";

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
      icon="subtitles"
      title={t("mobile.detail.retranscribe.action")}
      description={t("mobile.detail.retranscribe.body")}
      closeLabel={t("mobile.sheet.close")}
      handleLabel={t("mobile.sheet.handle")}
      footer={
        <>
          <PhoneButton disabled={busy} aria-busy={busy} onClick={() => void confirm()}>
            {t("mobile.detail.retranscribe.confirm")}
          </PhoneButton>
          <PhoneButton variant="ghost" size="compact" onClick={close}>
            {t("mobile.common.cancel")}
          </PhoneButton>
        </>
      }
    >
      <div className="flex flex-col gap-2">
        <span className="text-ios-footnote font-semibold text-muted">{t("mobile.detail.retranscribe.language")}</span>
        <ChoiceGroup
          label={t("mobile.detail.retranscribe.language")}
          value={language}
          onChange={setLanguage}
          options={[
            { value: "auto", label: t("mobile.import.lang.auto") },
            { value: "en", label: t("mobile.import.lang.en") },
            { value: "vi", label: t("mobile.import.lang.vi") },
          ]}
        />
        {error && (
          <p role="alert" className="text-ios-footnote m-0 text-warn">
            {t("system.commandFailed", { message: error })}
          </p>
        )}
      </div>
    </Sheet>
  );
}
