// SPDX-License-Identifier: Apache-2.0
// Feeds core events into the processing store and says "Notes are ready".
// Safe to mount more than once (the toast is deduped per meeting + version).
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useToast } from "@ghi/ui";
import { ipc } from "../../ipc";
import { useProcessing } from "./processing-store";

const toasted = new Set<string>();

async function notify(meeting: string, title: string) {
  const list = await ipc.commands.listMeetings(50, 0);
  const body = (list.status === "ok" && list.data.find((m) => m.gid === meeting)?.title) || "";
  await ipc.commands.showNotification(title, body);
}

export function ProcessingWatcher() {
  const { t } = useTranslation();
  const { show } = useToast();
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void ipc
      .onCoreEvent((env) => {
        useProcessing.getState().apply(env.event);
        if (env.event.type !== "notesReady" || env.event.version < 2) return;
        const key = `${env.event.meeting}:${env.event.version}`;
        if (toasted.has(key)) return;
        toasted.add(key);
        show({ tone: "success", title: t("processing.notesReady") });
        // The main window is hidden (the app lives in the menu bar): a toast alone would be missed.
        if (document.hidden) void notify(env.event.meeting, t("processing.notesReady"));
      })
      .then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  }, [show, t]);
  return null;
}
