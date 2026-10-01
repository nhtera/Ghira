// SPDX-License-Identifier: Apache-2.0
// Speaker commands with their toasts: naming, merging, splitting, "not a
// person". Each returns whether it worked; a failure says why in a toast.
import { useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useToast } from "@ghi/ui";
import { ipc } from "../../ipc";
import { useLive } from "../../state/live";
import { useSpeakerLabel } from "../../state/speaker-label";

type Result = { status: "ok" } | { status: "error"; error: string };

export function useSpeakerActions() {
  const { t } = useTranslation();
  const { show } = useToast();
  const labelOf = useSpeakerLabel();
  const nameOf = useCallback(
    (id: number, fallback = "") => {
      const s = useLive.getState().speakers[id];
      return s ? labelOf(s) : fallback;
    },
    [labelOf],
  );
  const check = useCallback(
    (r: Result): boolean => {
      if (r.status === "error") show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
      return r.status === "ok";
    },
    [show, t],
  );

  return useMemo(() => {
    const info = (title: string) => show({ tone: "success", title });
    return {
      async rename(id: number, name: string): Promise<boolean> {
        const value = name.trim();
        if (!value) return false;
        const ok = check(await ipc.commands.renameSpeaker(id, value));
        if (ok) info(t("speakers.renamed", { name: value }));
        return ok;
      },
      async merge(from: number, into: number): Promise<boolean> {
        const name = nameOf(into);
        const ok = check(await ipc.commands.mergeSpeakers(from, into));
        if (ok) info(t("speakers.merged", { name }));
        return ok;
      },
      async notPerson(id: number): Promise<boolean> {
        const name = nameOf(id);
        const ok = check(await ipc.commands.speakerNotAPerson(id));
        if (ok) info(t("speakers.notPerson.done", { name }));
        return ok;
      },
      /**
       * Moves lines (by gid) to a new speaker, optionally named; the
       * `speakerSplit` event moves them in the live store. Returns the new
       * speaker's id, or null when nothing moved.
       */
      async split(from: number, gids: string[], name = ""): Promise<number | null> {
        const r = await ipc.commands.splitSpeaker(from, gids);
        if (r.status === "error") {
          check(r);
          return null;
        }
        if (r.data == null) {
          show({ tone: "warning", title: t("speakers.split.none") });
          return null;
        }
        const id = r.data;
        const label = name.trim();
        if (label) check(await ipc.commands.renameSpeaker(id, label));
        info(t("speakers.split.done", { count: gids.length, name: label || nameOf(id, t("speakers.numbered", { number: id })) }));
        return id;
      },
    };
  }, [check, nameOf, show, t]);
}
