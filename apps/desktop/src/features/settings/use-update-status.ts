// SPDX-License-Identifier: Apache-2.0
// The app-update status: read once, then kept current by the core's events.
import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { UpdateStatus } from "../../bindings";
import { ipc } from "../../ipc";

export const updateKey = ["update-status"] as const;

export function useUpdateStatus() {
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: updateKey, queryFn: () => ipc.commands.updateStatus() });
  useEffect(() => {
    let alive = true;
    let off: (() => void) | undefined;
    void ipc
      .onUpdateChanged((e) => queryClient.setQueryData<UpdateStatus>(updateKey, e.status))
      .then((u) => (alive ? (off = u) : u()));
    return () => {
      alive = false;
      off?.();
    };
  }, [queryClient]);
  return query.data;
}
