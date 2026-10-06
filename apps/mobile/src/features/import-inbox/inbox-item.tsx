// SPDX-License-Identifier: Apache-2.0
// One waiting file in the M5 sheet: pick the language, then import (imported
// files are processed on this phone); or remove it. Importing and rejected
// items only report.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type {
  InboxItem as Item,
  MeetingLanguage,
  ProcessingTarget,
} from "../../bindings";
import { formatBytes } from "../models/format-bytes";
import { knownError } from "../settings/api";
import { Btn } from "../settings/controls";
import { ChoiceGroup } from "./choice-group";

export function InboxRow({
  item,
  onImport,
  onDismiss,
  error,
}: {
  item: Item;
  onImport: (language: MeetingLanguage, target: ProcessingTarget) => void;
  onDismiss: () => void;
  error: string | null;
}) {
  const { t, i18n } = useTranslation();
  const [language, setLanguage] = useState<MeetingLanguage>(item.language);
  // Never the computer: imported files are processed on this phone.
  const target: ProcessingTarget = item.target === "cloud" ? "cloud" : "phone";
  const size =
    item.sizeBytes == null
      ? t("mobile.inbox.unknownSize")
      : formatBytes(item.sizeBytes, i18n.language);
  const reason = item.reason ? knownError(item.reason) : "generic";

  return (
    <li
      data-state={item.state}
      className="flex flex-col gap-2 border-b border-line py-3 last:border-b-0"
    >
      <div className="flex flex-col">
        {/* The file name is untrusted text: rendered as a text node only. */}
        <span className="text-ios-body break-words">{item.name}</span>
        <span className="text-ios-footnote text-muted">{size}</span>
      </div>
      {item.state === "importing" && (
        <p role="status" className="text-ios-subhead m-0 text-muted">
          {t("mobile.inbox.importing")}
        </p>
      )}
      {item.state === "rejected" && (
        <>
          <p role="alert" className="text-ios-subhead m-0 text-rec-ink">
            {t("mobile.inbox.rejected")}: {t(`mobile.settings.error.${reason}`)}
          </p>
          <Btn
            aria-label={t("mobile.inbox.dismissFile", { name: item.name })}
            onClick={onDismiss}
          >
            {t("mobile.inbox.dismiss")}
          </Btn>
        </>
      )}
      {item.state === "pending" && (
        <>
          <ChoiceGroup
            label={t("mobile.import.language")}
            value={language}
            onChange={setLanguage}
            options={[
              { value: "auto", label: t("mobile.import.lang.auto") },
              { value: "en", label: t("mobile.import.lang.en") },
              { value: "vi", label: t("mobile.import.lang.vi") },
            ]}
          />
          {/* An imported file's audio never travels to the computer: it is
              processed here, whatever the record target is. */}
          <p className="text-ios-footnote m-0 text-muted">
            {t("mobile.inbox.processedOnPhone")}
          </p>
          {error && (
            <p role="alert" className="text-ios-footnote m-0 text-rec-ink">
              {t(`mobile.settings.error.${knownError(error)}`)}
            </p>
          )}
          <div className="flex gap-2">
            <Btn
              tone="primary"
              className="flex-1"
              onClick={() => onImport(language, target)}
            >
              {t("mobile.import.action")}
            </Btn>
            <Btn
              aria-label={t("mobile.inbox.dismissFile", { name: item.name })}
              onClick={onDismiss}
            >
              {t("mobile.inbox.dismiss")}
            </Btn>
          </div>
        </>
      )}
    </li>
  );
}
