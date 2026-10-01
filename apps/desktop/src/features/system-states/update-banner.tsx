// SPDX-License-Identifier: Apache-2.0
// "An update is ready" (the updater itself is phase 12; nothing mounts this
// yet). Installing restarts the app, so while recording or while a meeting is
// being processed it waits (RT-12).
import { useTranslation } from "react-i18next";
import { Button } from "@ghi/ui";
import type { SessionState } from "../../bindings";
import { isActive } from "../../state/live";
import { SystemBanner } from "./system-banner";

/** Restarting is not allowed while a recording runs or notes are being written. */
export const updateDeferred = (state: SessionState, processing: boolean) => isActive(state) || state === "processing" || processing;

export function UpdateBanner({ version, onInstall, deferred = false, onDismiss }: { version: string; onInstall: () => void; deferred?: boolean; onDismiss?: () => void }) {
  const { t } = useTranslation();
  return (
    <SystemBanner
      id="update"
      tone="info"
      icon="system_update_alt"
      title={t("system.update.title", { version })}
      onDismiss={onDismiss}
      actions={
        <Button size="sm" variant="primary" disabled={deferred} onClick={onInstall}>
          {t("system.update.restart")}
        </Button>
      }
    >
      {deferred ? t("system.update.deferred") : t("system.update.body")}
    </SystemBanner>
  );
}
