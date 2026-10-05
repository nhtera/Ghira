// SPDX-License-Identifier: Apache-2.0
// "Delete everything" with paired devices (doc 07 §7.9): first "Also delete on
// your paired devices?", then, while the core waits for a device that is out
// of reach, "Waiting for iPhone… [Delete here only]". Unreached devices keep
// their copies and show "Unpaired by <name>" at their next attempt.
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Button, Icon } from "@ghi/ui";
import type { DeleteEverywhereStatus } from "../../bindings";
import { ipc } from "../../ipc";

export function DeleteEverywhereAsk({ devices, onEverywhere, onHereOnly, onCancel }: { devices: string[]; onEverywhere: () => void; onHereOnly: () => void; onCancel: () => void }) {
  const { t } = useTranslation();
  return (
    <div role="group" aria-label={t("settings.sync.deleteEverywhereAsk")} className="flex max-w-lg flex-col gap-2.5 rounded-row border-[1.5px] border-rec bg-rec-soft px-3.5 py-3">
      <b className="text-body font-semibold text-rec-ink">{t("settings.sync.deleteEverywhereAsk")}</b>
      <p className="text-small m-0 text-muted">
        {devices.join(", ")}. {t("settings.sync.deleteEverywhereBody")}
      </p>
      <div className="flex flex-wrap gap-2">
        <Button variant="danger" onClick={onEverywhere}>
          {t("settings.sync.deleteEverywhere")}
        </Button>
        <Button onClick={onHereOnly}>{t("settings.sync.deleteHereOnly")}</Button>
        <Button onClick={onCancel}>{t("common.cancel")}</Button>
      </div>
    </div>
  );
}

/** The core's wait for unreachable devices, polled while `active`. */
export function useDeleteEverywhereStatus(active: boolean): DeleteEverywhereStatus | undefined {
  const { data } = useQuery({
    queryKey: ["sync-delete-everywhere"],
    enabled: active,
    refetchInterval: active ? 1000 : false,
    queryFn: async () => {
      const r = await ipc.commands.syncDeleteEverywhereStatus();
      if (r.status === "error") throw new Error(r.error);
      return r.data;
    },
  });
  return active ? data : undefined;
}

export function DeleteWaiting({ names, onHereOnly }: { names: string[]; onHereOnly: () => void }) {
  const { t } = useTranslation();
  return (
    <div role="status" data-testid="delete-waiting" className="flex max-w-lg flex-wrap items-center gap-3 rounded-row border border-line2 bg-surface px-3.5 py-3">
      <Icon name="schedule" size={20} className="text-muted" />
      <span className="text-body min-w-40 flex-1">{t("settings.sync.deleteWaiting", { device: names.join(", ") })}</span>
      <Button onClick={onHereOnly}>{t("settings.sync.deleteHereOnly")}</Button>
    </div>
  );
}
