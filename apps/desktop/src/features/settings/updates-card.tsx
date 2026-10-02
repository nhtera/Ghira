// SPDX-License-Identifier: Apache-2.0
// About → Updates: the version, a manual check, the automatic-check switch and
// "Restart to update" (after a confirm: the app closes and opens again).
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, Icon, InlineConfirm } from "@ghi/ui";
import { APP_NAME, formatDate, formatTime, type Locale } from "@ghi/i18n";
import type { UpdateStatus } from "../../bindings";
import { ipc } from "../../ipc";
import { Card, Note, SwitchRow, useFail, useSettings } from "./parts";
import { updateKey, useUpdateStatus } from "./use-update-status";

export function UpdatesCard() {
  const { t, i18n } = useTranslation();
  const lang = i18n.language as Locale;
  const queryClient = useQueryClient();
  const fail = useFail();
  const status = useUpdateStatus();
  const { settings, patch } = useSettings();
  const { data: version } = useQuery({ queryKey: ["app-version"], queryFn: () => ipc.commands.appVersion() });
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState(false);
  const strict = settings?.strictOffline ?? false;

  const check = async () => {
    setBusy(true);
    const r = await ipc.commands.checkForUpdates();
    setBusy(false);
    if (r.status === "ok") queryClient.setQueryData<UpdateStatus>(updateKey, r.data);
    else fail(r.error);
  };
  const install = async () => {
    setAsking(false);
    const r = await ipc.commands.installUpdate();
    if (r.status === "error") fail(r.error);
  };

  const when = status?.lastCheck != null ? `${formatDate(status.lastCheck, lang)} ${formatTime(status.lastCheck, lang)}` : null;
  const checking = busy || !!status?.checking;
  const canCheck = !!status?.configured && !strict && !checking;
  const line = !status
    ? ""
    : status.checking
      ? t("settings.updates.checking")
      : status.ready && status.available
        ? t("settings.updates.ready", { version: status.available })
        : status.available
          ? t("settings.updates.available", { version: status.available })
          : status.lastCheck != null
            ? t("settings.updates.upToDate", { app: APP_NAME })
            : t("settings.updates.neverChecked");

  return (
    <Card title={t("settings.updates.title")}>
      {version && <p className="text-small m-0 text-muted">{t("settings.about.version", { app: version.app, core: version.core })}</p>}
      {status && (
        <>
          <p role="status" className="text-body m-0 flex items-center gap-2" data-testid="update-line">
            <Icon name={status.ready ? "update" : "check_circle"} size={18} className="text-accent" />
            {line}
          </p>
          {when && !status.checking && <p className="text-small m-0 text-muted">{t("settings.updates.lastChecked", { when })}</p>}
          {status.error && <Note icon="warning" tone="warn">{status.error}</Note>}
          {status.notesUrl && <p className="text-small m-0 break-all text-muted">{t("settings.updates.notes", { url: status.notesUrl })}</p>}
          {asking && status.available ? (
            <InlineConfirm
              icon="update"
              question={t("settings.updates.confirm", { app: APP_NAME, version: status.available })}
              confirmLabel={t("settings.updates.confirmButton")}
              onConfirm={() => void install()}
              onCancel={() => setAsking(false)}
            />
          ) : (
            <div className="flex flex-wrap items-center gap-2">
              {status.ready && status.available && (
                <Button variant="primary" icon="update" onClick={() => setAsking(true)}>
                  {t("settings.updates.restart")}
                </Button>
              )}
              <Button icon="refresh" disabled={!canCheck} onClick={() => void check()}>
                {t("settings.updates.check")}
              </Button>
            </div>
          )}
          {!status.configured ? <Note>{t("settings.updates.notConfigured")}</Note> : strict && <Note icon="cloud_off">{t("settings.updates.strictOffline", { app: APP_NAME })}</Note>}
        </>
      )}
      {settings && <SwitchRow label={t("settings.updates.auto")} hint={t("settings.updates.autoHint")} checked={settings.updateCheck} onChange={(v) => void patch({ updateCheck: v })} disabled={status ? !status.configured : false} />}
    </Card>
  );
}
