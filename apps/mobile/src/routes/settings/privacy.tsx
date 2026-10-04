// SPDX-License-Identifier: Apache-2.0
// Settings → Privacy and security: where data goes (network line), app lock,
// audio retention, export and delete everything, and the cloud entry.
import { ListRow, ListSection } from "@ghi/ui";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ipc } from "../../ipc";
import { useGo } from "../../features/settings/go";
import { unwrap, useResource } from "../../features/settings/api";
import { ChoiceRow, ErrorLine, Switch } from "../../features/settings/controls";
import { Page } from "../../features/settings/page";
import { useAppSettings } from "../../features/settings/use-settings";
import { DeleteAllSheet, ExportSheet } from "./privacy-sheets";

const LOCK_AFTER = [0, 1, 5, 15] as const;
const RETENTION = [0, 30, 90, 365] as const;
const startOfToday = () => new Date().setHours(0, 0, 0, 0);

/** Cloud requests made today (the request log keeps no content). */
const loadRequestsToday = async () => unwrap(await ipc.commands.cloudRequestLog(200)).filter((r) => (r.at ?? 0) >= startOfToday()).length;

export function PrivacyScreen() {
  const { t } = useTranslation();
  const go = useGo();
  const app = useAppSettings();
  const requests = useResource(loadRequestsToday);
  const [sheet, setSheet] = useState<"export" | "delete" | null>(null);
  const today = requests.data ?? 0;
  const s = app.settings;

  return (
    <Page title={t("mobile.privacy.title")} back="settings" error={app.loadError} onRetry={app.reload}>
      {s && (
        <>
          <ErrorLine code={app.saveError} fallback="mobile.settings.saveFailed" />
          <ListSection header={t("mobile.privacy.networkHeader")} footer={t("mobile.privacy.networkFooter")}>
            <ListRow
              title={`${t("mobile.privacy.localOnly")} · ${t("mobile.privacy.requestsToday", { count: today })}`}
              icon={today > 0 ? "cloud" : "lock"}
            />
          </ListSection>

          <ListSection header={t("mobile.privacy.lockHeader")}>
            <ListRow
              title={t("mobile.privacy.lockToggle")}
              subtitle={t("mobile.privacy.lockToggleHint")}
              trailing={(id) => (
                <Switch checked={s.appLock} labelledBy={id} onChange={(on) => void app.setLock(on, s.lockAfterMinutes, t("mobile.lock.unlockReason"))} />
              )}
            />
          </ListSection>
          {s.appLock && (
            <ListSection header={t("mobile.privacy.lockAfter")}>
              {LOCK_AFTER.map((m) => (
                <ChoiceRow key={m} title={t(`mobile.privacy.lockAfter${m}`)} selected={s.lockAfterMinutes === m} onPress={() => void app.setLock(true, m, t("mobile.lock.unlockReason"))} />
              ))}
            </ListSection>
          )}

          <ListSection header={t("mobile.privacy.retention")} footer={t("mobile.privacy.retentionFooter")}>
            {RETENTION.map((d) => (
              <ChoiceRow
                key={d}
                title={d === 0 ? t("mobile.privacy.retentionForever") : t(`mobile.privacy.retention${d}`)}
                selected={s.audioRetentionDays === d}
                onPress={() => void app.patch({ audioRetentionDays: d })}
              />
            ))}
          </ListSection>

          <ListSection header={t("mobile.privacy.dataHeader")}>
            <ListRow title={t("mobile.privacy.exportAll")} subtitle={t("mobile.privacy.exportAllHint")} icon="ios_share" onPress={() => setSheet("export")} />
            <ListRow title={t("mobile.privacy.cloudRow")} icon="cloud" chevron onPress={() => go("/settings/cloud")} />
            <ListRow title={t("mobile.privacy.logRow")} icon="cloud" chevron onPress={() => go("/settings/privacy/log")} />
            <ListRow title={t("mobile.privacy.deleteAll.row")} destructive icon="delete_forever" onPress={() => setSheet("delete")} />
          </ListSection>

          <ExportSheet open={sheet === "export"} onOpenChange={(o) => setSheet(o ? "export" : null)} />
          <DeleteAllSheet open={sheet === "delete"} onOpenChange={(o) => setSheet(o ? "delete" : null)} />
        </>
      )}
    </Page>
  );
}
