// SPDX-License-Identifier: Apache-2.0
// Speaker commands with their toasts: naming, merging, splitting, "not a
// person". Each returns whether it worked; a failure says why in a toast.
import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useToast } from "@ghi/ui";
import { ipc } from "../../ipc";
import { invalidateMeeting } from "../../state/meeting-queries";
import { useLive } from "../../state/live";
import { errorText } from "../people/error-text";
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

/**
 * The voice actions of a STORED meeting's speakers (the live strip has none):
 * accept or dismiss "Sounds like …", "This is me" and "Not me". Each returns
 * whether it worked; a failure says why in a toast (farSide: in a call only
 * the mic speaker can be Me). A success reloads the meeting.
 */
export function useStoredSpeakerActions(meeting: string) {
  const { t } = useTranslation();
  const { show } = useToast();
  const client = useQueryClient();
  return useMemo(() => {
    const run = async (call: Promise<Result>, done: string | null): Promise<boolean> => {
      const r = await call;
      if (r.status === "error") {
        show({ tone: "warning", title: errorText(t, r.error) });
        return false;
      }
      if (done) show({ tone: "success", title: done });
      void invalidateMeeting(client, meeting);
      void client.invalidateQueries({ queryKey: ["people"] });
      // Me is a name in the suggestions too.
      void client.invalidateQueries({ queryKey: ["knownSpeakerNames"] });
      return true;
    };
    return {
      thisIsMe: (speaker: string) => run(ipc.commands.setSpeakerMe(meeting, speaker), t("speakers.markedMe")),
      notMe: (speaker: string) => run(ipc.commands.clearSpeakerMe(meeting, speaker), t("speakers.unmarkedMe")),
      accept: (speaker: string, isMe: boolean, name: string) =>
        run(ipc.commands.acceptVoiceSuggestion(meeting, speaker), isMe ? t("speakers.markedMe") : t("speakers.renamed", { name })),
      dismiss: (speaker: string) => run(ipc.commands.dismissVoiceSuggestion(meeting, speaker), null),
    };
  }, [client, meeting, show, t]);
}
