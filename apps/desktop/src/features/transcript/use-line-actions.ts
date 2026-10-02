// SPDX-License-Identifier: Apache-2.0
// Edits to one transcript line: its text, or who said it. Each re-reads the
// meeting on success (the notes keep their citations) and says why on failure.
import { useQueryClient } from "@tanstack/react-query";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useToast } from "@ghi/ui";
import { ipc } from "../../ipc";
import { invalidateMeeting } from "../../state/meeting-queries";

export function useLineActions(meeting: string) {
  const { t } = useTranslation();
  const { show } = useToast();
  const client = useQueryClient();
  return useMemo(() => {
    const done = async (r: { status: "ok" } | { status: "error"; error: string }, saved: string): Promise<boolean> => {
      if (r.status === "error") {
        show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
        return false;
      }
      await invalidateMeeting(client, meeting);
      show({ tone: "success", title: saved });
      return true;
    };
    return {
      saveText: async (segment: string, text: string) => done(await ipc.commands.updateSegmentText(meeting, segment, text), t("speakers.line.saved")),
      setSpeaker: async (segment: string, speaker: string, name: string) => done(await ipc.commands.setSegmentSpeaker(meeting, segment, speaker), t("speakers.line.moved", { name })),
    };
  }, [meeting, client, show, t]);
}
