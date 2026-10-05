// SPDX-License-Identifier: Apache-2.0
// "Edited on <device>: [Use this] [Dismiss]": the losing side of a concurrent
// edit, kept as a conflict copy (doc 07 §7.5). Shows nothing without one.
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Button, Icon } from "@ghi/ui";
import { ipc } from "../../ipc";
import { invalidateMeeting } from "../../state/meeting-queries";
import { useFail } from "../settings/parts";
import { useSyncEvents } from "./use-sync";

const key = (meeting: string) => ["sync-conflicts", meeting] as const;

export function ConflictBanner({ meeting }: { meeting: string }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const fail = useFail();
  const { data } = useQuery({
    queryKey: key(meeting),
    queryFn: async () => {
      const r = await ipc.commands.syncConflicts(meeting);
      return r.status === "ok" ? r.data : [];
    },
  });
  useSyncEvents((e) => {
    if (e.type === "conflict" && e.meeting === meeting) void client.invalidateQueries({ queryKey: key(meeting) });
  });
  const copy = data?.[0];
  if (!copy) return null;

  const resolve = async (useIt: boolean) => {
    const r = await ipc.commands.syncConflictResolve(copy.gid, useIt);
    if (r.status === "error") return fail(r.error);
    await client.invalidateQueries({ queryKey: key(meeting) });
    if (useIt) await invalidateMeeting(client, meeting);
  };

  return (
    <div role="status" data-testid="conflict-banner" className="mx-7 mt-2 flex flex-wrap items-center gap-3 rounded-panel border-[1.5px] border-line2 bg-accent-soft p-3.5">
      <Icon name="call_merge" size={20} className="shrink-0 text-accent" />
      <div className="min-w-48 flex-1">
        <b className="text-body font-semibold">{t("settings.sync.editedOn", { device: copy.device })}</b>
        <p className="text-small m-0 text-muted">{copy.text}</p>
      </div>
      <Button size="sm" variant="primary" onClick={() => void resolve(true)}>
        {t("settings.sync.useThis")}
      </Button>
      <Button size="sm" onClick={() => void resolve(false)}>
        {t("settings.sync.dismiss")}
      </Button>
    </div>
  );
}
