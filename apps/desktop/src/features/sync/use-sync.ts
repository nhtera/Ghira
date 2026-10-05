// SPDX-License-Identifier: Apache-2.0
// The Sync UI's view of the core: the status query (kept current by sync
// events) and one subscription helper.
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import type { SyncEvent, SyncStatus } from "../../bindings";
import { ipc } from "../../ipc";

export const SYNC_KEY = ["sync-status"] as const;

export const syncStatusQuery = {
  queryKey: SYNC_KEY,
  queryFn: async (): Promise<SyncStatus> => {
    const r = await ipc.commands.syncStatus();
    if (r.status === "error") throw new Error(r.error);
    return r.data;
  },
};

/** Calls `cb` for every sync event while mounted (the latest `cb`, no resubscribe). */
export function useSyncEvents(cb: (e: SyncEvent) => void) {
  const latest = useRef(cb);
  useEffect(() => {
    latest.current = cb;
  });
  useEffect(() => {
    let dead = false;
    let off: (() => void) | undefined;
    void ipc.onSyncEvent((e) => latest.current(e)).then((u) => {
      if (dead) u();
      else off = u;
    });
    return () => {
      dead = true;
      off?.();
    };
  }, []);
}

/** The status, re-read on every sync event; `pollMs` adds a timer (a state no event announces). */
export function useSyncStatus(pollMs?: number) {
  const client = useQueryClient();
  const q = useQuery({ ...syncStatusQuery, refetchInterval: pollMs ?? false });
  useSyncEvents(() => void client.invalidateQueries({ queryKey: SYNC_KEY }));
  return q;
}
