// SPDX-License-Identifier: Apache-2.0
// "Transcribe again…" (Export menu): the panel takes the menu row's place under
// the toolbar, like Regenerate's question, with the spoken language to use.
// The final pass runs again on the stored audio; names, edited lines and the
// user's notes stay, and the AI notes are written again after it.
import { Button, Icon, cn, useToast } from "@ghi/ui";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingDetail, TranscriptLanguage } from "../../bindings";
import { ipc } from "../../ipc";
import { invalidateMeeting } from "../../state/meeting-queries";

/** What the meeting was transcribed in, as the panel's first choice. */
export function initialLanguage(detail: MeetingDetail): TranscriptLanguage {
  return detail.language === "en" || detail.language === "vi" ? detail.language : "auto";
}

export function RetranscribePanel({ detail, onClose }: { detail: MeetingDetail; onClose: () => void }) {
  const { t } = useTranslation();
  const { show } = useToast();
  const client = useQueryClient();
  const id = useId();
  const [language, setLanguage] = useState<TranscriptLanguage>(() => initialLanguage(detail));
  const [busy, setBusy] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);
  useEffect(() => cancelRef.current?.focus(), []);

  const options: { value: TranscriptLanguage; label: string }[] = [
    { value: "auto", label: t("meeting.retranscribeAuto") },
    { value: "en", label: t("import.options.languages.english") },
    { value: "vi", label: t("import.options.languages.vietnamese") },
  ];

  const confirm = async () => {
    setBusy(true);
    const r = await ipc.commands.retranscribe(detail.gid, language);
    setBusy(false);
    if (r.status === "error") {
      show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
      return;
    }
    // true: waits for the speech models; false: runs now.
    show({ tone: "info", title: r.data ? t("meeting.retranscribeWaiting") : t("meeting.retranscribing") });
    onClose();
    void invalidateMeeting(client, detail.gid);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      onClose();
    }
  };

  return (
    <div
      role="alertdialog"
      aria-labelledby={id}
      onKeyDown={onKeyDown}
      className="flex flex-wrap items-center gap-2.5 rounded-row border-[1.5px] border-warn bg-warn-soft px-3.5 py-3"
    >
      <Icon name="subtitles" size={20} className="text-warn" />
      <b id={id} className="text-body min-w-48 flex-1 font-semibold text-warn">
        {t("meeting.retranscribeQuestion")}
      </b>
      <div role="radiogroup" aria-label={t("meeting.retranscribeLanguage")} className="flex gap-0.5 rounded-ctl border border-ctl bg-surface p-0.5">
        {options.map((o) => (
          <button
            key={o.value}
            type="button"
            role="radio"
            aria-checked={language === o.value}
            onClick={() => setLanguage(o.value)}
            className={cn("h-6 rounded-seg px-[9px] text-[12px] font-semibold", language === o.value ? "bg-accent-soft text-accent" : "text-muted hover:text-ink")}
          >
            {o.label}
          </button>
        ))}
      </div>
      <Button variant="danger" className="bg-warn text-surface" disabled={busy} onClick={() => void confirm()}>
        {t("meeting.retranscribeConfirm")}
      </Button>
      <Button ref={cancelRef} onClick={onClose}>
        {t("common.cancel")}
      </Button>
    </div>
  );
}
