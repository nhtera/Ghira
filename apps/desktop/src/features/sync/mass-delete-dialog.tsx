// SPDX-License-Identifier: Apache-2.0
// "<device> deleted 12 meetings. Delete them here too?": the other device's
// mass delete waits for this answer (SyncEvent::NeedsConfirm, doc 07 D13).
// Mount it once where it is always visible; the pending question is kept in
// a store so a late mount still shows it.
import { create } from "zustand";
import { useTranslation } from "react-i18next";
import { Button, Dialog } from "@ghi/ui";
import { ipc } from "../../ipc";
import { useFail } from "../settings/parts";
import { useSyncEvents } from "./use-sync";

type Ask = { device: string; count: number };
export const useMassDelete = create<{ ask: Ask | null }>(() => ({ ask: null }));

export function MassDeleteDialog() {
  const { t } = useTranslation();
  const fail = useFail();
  const ask = useMassDelete((s) => s.ask);
  useSyncEvents((e) => {
    if (e.type === "needsConfirm") useMassDelete.setState({ ask: { device: e.device, count: e.count } });
  });

  const answer = async (accept: boolean) => {
    const r = await ipc.commands.syncConfirmMassDelete(accept);
    if (r.status === "error") return fail(r.error);
    useMassDelete.setState({ ask: null });
  };

  return (
    <Dialog
      open={ask != null}
      // Nothing is deleted until the user chooses: no Escape or scrim dismissal.
      dismissible={false}
      onOpenChange={() => {}}
      title={ask ? t("settings.sync.massDeleteTitle", { device: ask.device, count: ask.count }) : ""}
      description={t("settings.sync.massDeleteBody")}
      footer={
        <>
          <Button onClick={() => void answer(false)}>{t("settings.sync.massDeleteKeep")}</Button>
          <Button variant="danger" onClick={() => void answer(true)}>
            {t("settings.sync.massDeleteApply")}
          </Button>
        </>
      }
    />
  );
}
