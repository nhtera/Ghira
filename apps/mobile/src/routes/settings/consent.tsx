// SPDX-License-Identifier: Apache-2.0
// Settings → Consent message: the text to read out when a recording starts,
// in English and Vietnamese. Empty means the built-in text.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { AppSettings, ConsentMessage } from "../../bindings";
import { ipc } from "../../ipc";
import { unwrap, useResource } from "../../features/settings/api";
import { Btn, ErrorLine } from "../../features/settings/controls";
import { Page } from "../../features/settings/page";
import { useAppSettings } from "../../features/settings/use-settings";

const loadBuiltIn = async () => unwrap(await ipc.commands.recordConsentMessage("auto"));

const AREA = "text-ios-body w-full rounded-(--ios-radius-group) border border-ctl bg-surface p-3 text-ink select-text";

export function ConsentScreen() {
  const { t } = useTranslation();
  const app = useAppSettings();
  const builtIn = useResource(loadBuiltIn);
  return (
    <Page title={t("mobile.settings.consent.title")} back="settings" error={app.loadError ?? builtIn.error} onRetry={() => (app.reload(), builtIn.reload())}>
      {app.settings && builtIn.data && <ConsentForm settings={app.settings} builtIn={builtIn.data} patch={app.patch} error={app.saveError} />}
    </Page>
  );
}

function ConsentForm({ settings, builtIn, patch, error }: { settings: AppSettings; builtIn: ConsentMessage; patch: ReturnType<typeof useAppSettings>["patch"]; error: string | null }) {
  const { t } = useTranslation();
  const [en, setEn] = useState(settings.consentMessageEn);
  const [vi, setVi] = useState(settings.consentMessageVi);
  const [saved, setSaved] = useState(false);
  const dirty = en !== settings.consentMessageEn || vi !== settings.consentMessageVi;
  const save = async (next: { en: string; vi: string }) => {
    setSaved(false);
    if (await patch({ consentMessageEn: next.en, consentMessageVi: next.vi })) setSaved(true);
  };
  return (
    <div className="mx-4 flex flex-col gap-3">
      <p className="text-ios-footnote m-0 text-muted">{t("mobile.settings.consent.footer")}</p>
      <ErrorLine code={error} fallback="mobile.settings.saveFailed" />
      <label className="flex flex-col gap-1">
        <span className="text-ios-footnote px-1 font-medium text-muted">{t("mobile.settings.consent.english")}</span>
        <textarea lang="en" rows={4} className={AREA} value={en} placeholder={builtIn.en} onChange={(e) => setEn(e.target.value)} />
      </label>
      <label className="flex flex-col gap-1">
        <span className="text-ios-footnote px-1 font-medium text-muted">{t("mobile.settings.consent.vietnamese")}</span>
        <textarea lang="vi" rows={4} className={AREA} value={vi} placeholder={builtIn.vi} onChange={(e) => setVi(e.target.value)} />
      </label>
      <Btn tone="primary" disabled={!dirty} onClick={() => void save({ en, vi })}>
        {t("mobile.settings.consent.save")}
      </Btn>
      <Btn
        disabled={!en && !vi}
        onClick={() => {
          setEn("");
          setVi("");
          void save({ en: "", vi: "" });
        }}
      >
        {t("mobile.settings.consent.reset")}
      </Btn>
      {saved && !dirty && (
        <p role="status" className="text-ios-footnote m-0 text-accent">
          {t("mobile.settings.consent.saved")}
        </p>
      )}
    </div>
  );
}
