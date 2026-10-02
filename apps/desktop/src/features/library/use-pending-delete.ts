// SPDX-License-Identifier: Apache-2.0
// Delete with Undo: the rows disappear at once, the real `deleteMeeting` runs
// after UNDO_MS unless Undo is pressed. A refusal (a job is running) brings
// the row back and shows the error.
import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { useToast } from "@ghi/ui";
import { ipc } from "../../ipc";
import { MEETINGS_KEY } from "./use-meetings";

export const UNDO_MS = 6000;

export function usePendingDelete(onDeleted?: (id: string) => void) {
  const { t } = useTranslation();
  const { show } = useToast();
  const client = useQueryClient();
  const [hidden, setHidden] = useState<ReadonlySet<string>>(new Set());

  const unhide = useCallback((ids: string[]) => setHidden((h) => new Set([...h].filter((x) => !ids.includes(x)))), []);

  const schedule = useCallback(
    (ids: string[]) => {
      if (ids.length === 0) return;
      setHidden((h) => new Set([...h, ...ids]));
      let committed = false;
      const timer = setTimeout(async () => {
        committed = true;
        const failed: string[] = [];
        let error = "";
        for (const id of ids) {
          const r = await ipc.commands.deleteMeeting(id);
          if (r.status === "error") {
            failed.push(id);
            error = r.error;
          } else onDeleted?.(id);
        }
        if (failed.length) {
          unhide(failed);
          show({
            tone: "warning",
            title: t("system.commandFailed", { message: error }),
          });
        }
        // Rows that went stay hidden until the refetched list no longer has them.
        await client.invalidateQueries({ queryKey: MEETINGS_KEY });
        unhide(ids.filter((id) => !failed.includes(id)));
      }, UNDO_MS);
      show({
        title: t("library.deleted", { count: ids.length }),
        action: {
          label: t("common.undo"),
          altText: t("common.undo"),
          onAction: () => {
            // The toast can outlive the timer (Radix pauses it on hover): too late to undo.
            if (committed) return show({ tone: "warning", title: t("library.undoTooLate") });
            clearTimeout(timer);
            unhide(ids);
          },
        },
      });
    },
    [client, show, t, unhide, onDeleted],
  );

  return { hidden, schedule };
}
