// SPDX-License-Identifier: Apache-2.0
// The panel's commands on a stored meeting. Nothing is emitted by the core, so
// every success reloads the meeting; a failure comes back as its code for the
// panel to say in words (no toast: the sentence sits next to the action).
import { useQueryClient } from "@tanstack/react-query";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useToast } from "@ghi/ui";
import { ipc } from "../../ipc";
import { invalidateMeeting } from "../../state/meeting-queries";
import type { SplitChoice } from "./logic";
import { splitArgs } from "./logic";

export type Outcome = { ok: true } | { ok: false; error: string };
type Raw = { status: "ok" } | { status: "error"; error: string };

export function usePanelActions(meeting: string) {
  const { t } = useTranslation();
  const { show } = useToast();
  const client = useQueryClient();
  return useMemo(() => {
    const run = async (call: Promise<Raw>, done: string): Promise<Outcome> => {
      const r = await call;
      if (r.status === "error") return { ok: false, error: r.error };
      show({ tone: "success", title: done });
      void invalidateMeeting(client, meeting);
      void client.invalidateQueries({ queryKey: ["people"] });
      void client.invalidateQueries({ queryKey: ["knownSpeakerNames"] });
      return { ok: true };
    };
    return {
      rename: (speaker: string, name: string) =>
        run(ipc.commands.renameMeetingSpeaker(meeting, speaker, name), name.trim() ? t("speakers.renamed", { name: name.trim() }) : t("speakerPanel.nameCleared")),
      setMe: (speaker: string, me: boolean) =>
        run(me ? ipc.commands.setSpeakerMe(meeting, speaker) : ipc.commands.clearSpeakerMe(meeting, speaker), me ? t("speakers.markedMe") : t("speakers.unmarkedMe")),
      merge: (from: string, into: string, intoName: string) => run(ipc.commands.mergeMeetingSpeakers(meeting, from, into), t("speakers.merged", { name: intoName })),
      notPerson: (speaker: string, notPerson: boolean, name: string) =>
        run(ipc.commands.setSpeakerNotPerson(meeting, speaker, notPerson), notPerson ? t("speakers.notPerson.done", { name }) : t("speakerPanel.isPerson", { name })),
      split: (speaker: string, choice: SplitChoice, count: number) => {
        const { segmentGids, fromSegment } = splitArgs(choice);
        return run(ipc.commands.splitMeetingSpeaker(meeting, speaker, segmentGids, fromSegment), t("speakerPanel.splitDone", { count }));
      },
    };
  }, [client, meeting, show, t]);
}
