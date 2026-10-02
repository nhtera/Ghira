// SPDX-License-Identifier: Apache-2.0
// Settings → Recording: detection, the microphone permission, the consent
// message. Echo cancellation and per-app capture have no setting (call mode
// handles them), so they are shown as information only.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { Button, Icon, usePlatform } from "@ghi/ui";
import { APP_NAME } from "@ghi/i18n";
import { ipc } from "../../ipc";
import { Card, Note, Row, SwitchRow, inputCls, useSettings } from "./parts";

export function RecordingSection() {
  const { t } = useTranslation();
  const context = usePlatform();
  const { settings, patch } = useSettings();
  const { data: mic } = useQuery({ queryKey: ["mic-permission"], queryFn: () => ipc.commands.micPermission() });
  return (
    <div className="flex flex-col gap-4">
      <Card>
        {settings && (
          <SwitchRow label={t("settings.recording.detectApps")} hint={t("settings.recording.neverAuto", { app: APP_NAME })} checked={settings.detectMeetings} onChange={(v) => void patch({ detectMeetings: v })} />
        )}
        <Row label={t("settings.recording.micTitle")} hint={mic && t(`settings.recording.mic.${mic}`)}>
          {mic && mic !== "unsupported" && mic !== "granted" && (
            <Button onClick={() => void ipc.commands.openPrivacySettings("microphone")}>{t("settings.recording.openPrivacy", { context })}</Button>
          )}
          {mic === "granted" && <Icon name="check_circle" size={20} className="text-accent" />}
        </Row>
        <Note icon="check_circle">{t("settings.recording.echoOn")}</Note>
      </Card>
      <Card title={t("settings.recording.consentMessage")} hint={t("settings.recording.consentMessageHint")}>
        {settings && (
          <>
            <ConsentEditor lang="en" value={settings.consentMessageEn} onSave={(v) => patch({ consentMessageEn: v })} />
            <ConsentEditor lang="vi" value={settings.consentMessageVi} onSave={(v) => patch({ consentMessageVi: v })} />
          </>
        )}
      </Card>
    </div>
  );
}

function ConsentEditor({ lang, value, onSave }: { lang: "en" | "vi"; value: string; onSave: (v: string) => Promise<unknown> }) {
  const { t } = useTranslation();
  // null: not edited, show the saved value (so a reset or reload shows through).
  const [draft, setDraft] = useState<string | null>(null);
  const text = draft ?? value;
  const label = t(lang === "en" ? "settings.recording.consentEn" : "settings.recording.consentVi");
  const builtin = t("live.consent.message", { app: APP_NAME, lng: lang });
  return (
    <div className="flex flex-col gap-1.5">
      <label className="text-small font-semibold" htmlFor={`consent-${lang}`}>
        {label}
      </label>
      <textarea
        id={`consent-${lang}`}
        lang={lang}
        rows={3}
        className={`${inputCls} h-auto resize-y py-1.5`}
        value={text}
        placeholder={builtin}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => {
          if (text.trim() !== value) void onSave(text.trim());
          setDraft(null);
        }}
      />
      <div>
        <Button
          size="sm"
          variant="ghost"
          disabled={!value && !text}
          onClick={() => {
            setDraft(null);
            void onSave("");
          }}
        >
          {t("settings.recording.consentReset")}
        </Button>
      </div>
    </div>
  );
}
