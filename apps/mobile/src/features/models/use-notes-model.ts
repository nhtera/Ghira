// SPDX-License-Identifier: Apache-2.0
// The notes model on this phone: `null` when the phone cannot write notes
// itself (below 8 GB), else its row, kept current by the download events.
import { useEffect, useState } from "react";
import type { MobileModelItem } from "../../bindings";
import { ipc } from "../../ipc";
import { unwrap, useResource } from "../settings/api";

const load = async () => unwrap(await ipc.commands.notesModelStatus());

export function useNotesModel() {
  const status = useResource(load);
  const [live, setLive] = useState<MobileModelItem | null>(null);

  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    void ipc
      .onMobileEvent((e) => {
        if (e.type === "modelDownload" && e.item.role === "notes") setLive(e.item);
      })
      .then((u) => (alive ? (off = u) : u()));
    return () => {
      alive = false;
      off?.();
    };
  }, []);

  const item = status.data === undefined ? undefined : status.data && (live ?? status.data);
  return {
    /** `undefined` while loading, `null` on a phone that cannot write notes. */
    item,
    ready: item?.state === "ready",
    reload: () => {
      setLive(null);
      status.reload();
    },
    error: status.error,
  };
}
