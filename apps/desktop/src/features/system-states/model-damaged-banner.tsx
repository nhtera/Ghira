// SPDX-License-Identifier: Apache-2.0
// A model file failed its checksum at load (D12): notes can't be written until
// it is downloaded again. Recordings are safe. Re-checked when the core
// reports an engine or job error.
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { formatBytes } from "@ghi/i18n";
import { Button } from "@ghi/ui";
import { ipc } from "../../ipc";
import { useModelDownload } from "../onboarding/use-model-download";
import { BANNER_ACTION, SystemBanner } from "./system-banner";

export function ModelDamagedBanner() {
  const { t, i18n } = useTranslation();
  const dl = useModelDownload();
  const { refresh } = dl;

  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    void ipc
      .onCoreEvent((e) => {
        if (e.event.type === "error" && (e.event.kind === "engine" || e.event.kind === "job" || e.event.kind === "modelsMissing")) refresh();
      })
      .then((u) => (alive ? (off = u) : u()));
    return () => {
      alive = false;
      off?.();
    };
  }, [refresh]);

  const damaged = dl.models.filter((m) => m.model.damaged);
  const active = dl.models.find((m) => m.active);
  // Stays up (with progress) until the status says the file is good again.
  if (damaged.length === 0) return null;
  const size = damaged.reduce((n, m) => n + (m.model.size ?? 0), 0);

  return (
    <SystemBanner
      id="model-damaged"
      icon="error"
      actions={
        active ? (
          <span className="text-small font-semibold">{t("settings.models.status.downloading", { percent: active.percent })}</span>
        ) : (
          <Button size="sm" variant="ghost" className={BANNER_ACTION} onClick={dl.start}>
            {t("system.redownload", { size: formatBytes(size, i18n.language) })}
          </Button>
        )
      }
    >
      {t("system.modelDamaged")}
      {dl.error && !active && <span className="block">{t("system.commandFailed", { message: dl.error })}</span>}
    </SystemBanner>
  );
}
