// SPDX-License-Identifier: Apache-2.0
// Privacy → app lock: the switch (turning it on asks for Touch ID or the
// password first), when to lock, and "Lock now". The gate itself is
// shell/lock-gate.tsx.
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { Button, Select, usePlatform } from "@ghi/ui";
import { APP_NAME } from "@ghi/i18n";
import { lockErrorText } from "../../shell/lock-gate";
import { ipc } from "../../ipc";
import { settingsQuery } from "../../shell/root-view";
import { Card, Row, Switch, useFail, useSettings } from "./parts";

export const LOCK_MINUTES = [1, 5, 15, 60, 0] as const;
const DEFAULT_MINUTES = 5;

export function AppLockCard() {
  const { t } = useTranslation();
  const context = usePlatform();
  const fail = useFail();
  const queryClient = useQueryClient();
  const { settings } = useSettings();
  const id = useId();
  const lockAfterId = useId();
  if (!settings) return null;

  const apply = async (on: boolean, minutes: number) => {
    const r = await ipc.commands.setAppLock(on, minutes, t("system.locked.reasonChange"));
    if (r.status === "ok") queryClient.setQueryData(settingsQuery.queryKey, r.data);
    // A cancelled Touch ID / password prompt just leaves the lock as it was.
    else if (r.error !== "notConfirmed") fail(lockErrorText(t, r.error));
  };
  const lockMinutes = settings.lockAfterMinutes;

  return (
    <Card>
      <Row label={t("settings.privacy.lock", { context, app: APP_NAME })} hint={`${t("settings.privacy.lockNote")} ${t("settings.privacy.lockEncrypted")}`} id={id}>
        <Switch checked={settings.appLock} onChange={(on) => void apply(on, on && !lockMinutes && !settings.appLock ? DEFAULT_MINUTES : lockMinutes)} labelledBy={id} />
      </Row>
      {settings.appLock && (
        <div data-row className="flex flex-wrap items-center justify-between gap-3 border-b border-line py-3.5">
          <span className="text-body flex items-center gap-3">
            <label htmlFor={lockAfterId}>{t("settings.privacy.lockAfter")}</label>
            <Select
              id={lockAfterId}
              value={String(lockMinutes)}
              onChange={(v) => void apply(true, Number(v))}
              options={LOCK_MINUTES.map((m) => ({ value: String(m), label: t(`settings.privacy.lockAfterOptions.${m}`) }))}
            />
          </span>
          <Button icon="lock" onClick={() => void ipc.commands.lockNow()}>
            {t("settings.privacy.lockNow")}
          </Button>
        </div>
      )}
    </Card>
  );
}
