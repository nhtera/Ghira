// SPDX-License-Identifier: Apache-2.0
// M2 consent reminder, before every recording: everyone present should know.
// Offers a short message to copy; starting needs the explicit confirm, so the
// closing the sheet never starts one. With a phone call
// active the M6 notice takes its place (CallNoticeSheet).
import { PhoneButton, Sheet } from "@ghi/ui";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingLanguage } from "../../bindings";
import { ipc } from "../../ipc";
import { SensitiveRow } from "../sensitive";

/** The clipboard, if the webview lets us (it can refuse); resolves whether it worked. */
export async function writeClipboard(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/**
 * "Copy message": the consent text for the clipboard, "Copied" until the sheet
 * closes (it mounts with the sheet). The text is fetched when the sheet opens so
 * the tap itself writes to the clipboard (WebKit wants it inside the gesture).
 */
export function CopyConsentButton({ language, className }: { language: MeetingLanguage; className?: string }) {
  const { t } = useTranslation();
  const [message, setMessage] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    let alive = true;
    void ipc.commands.recordConsentMessage(language).then((r) => alive && r.status === "ok" && setMessage(r.data.text));
    return () => {
      alive = false;
    };
  }, [language]);
  return (
    <PhoneButton
      variant="secondary"
      icon={copied ? "check" : "content_copy"}
      disabled={message === null}
      onClick={() =>
        message !== null && void writeClipboard(message).then(setCopied)
      }
      className={className}
    >
      {copied
        ? t("mobile.record.consent.copied")
        : t("mobile.record.consent.copy")}
    </PhoneButton>
  );
}

export type ConsentSheetProps = {
  language: MeetingLanguage;
  open: boolean;
  onCancel: () => void;
  onConfirm: () => void;
  /** The "Sensitive meeting" choice for this recording (none: not offered). */
  sensitive?: { checked: boolean; onChange: (on: boolean) => void };
};

export function ConsentSheet({ language, open, onCancel, onConfirm, sensitive }: ConsentSheetProps) {
  const { t } = useTranslation();
  return (
    <Sheet
      open={open}
      onOpenChange={(o) => !o && onCancel()}
      icon="campaign"
      title={t("mobile.record.consent.title")}
      description={t("mobile.record.consent.body")}
      closeLabel={t("mobile.sheet.close")}
      handleLabel={t("mobile.sheet.handle")}
      footer={
        <>
          <PhoneButton onClick={onConfirm}>
            {t("mobile.record.consent.confirm")}
          </PhoneButton>
          <PhoneButton variant="ghost" onClick={onCancel}>
            {t("mobile.common.cancel")}
          </PhoneButton>
        </>
      }
    >
      <CopyConsentButton language={language} />
      {sensitive && (
        <div className="mt-3 rounded-(--ios-radius-group) bg-sunk px-4 py-3">
          <SensitiveRow checked={sensitive.checked} onChange={sensitive.onChange} />
        </div>
      )}
    </Sheet>
  );
}
