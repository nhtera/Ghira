// SPDX-License-Identifier: Apache-2.0
// Feeds core events into the processing store and says "Notes are ready".
// Safe to mount more than once (the toast is deduped per meeting + version).
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useToast } from "@ghi/ui";
import { ipc } from "../../ipc";
import { useProcessing } from "./processing-store";

const toasted = new Set<string>();

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
      })
      .then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  }, [show, t]);
  return null;
}
