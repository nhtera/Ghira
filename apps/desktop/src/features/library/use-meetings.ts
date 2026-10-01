// SPDX-License-Identifier: Apache-2.0
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import type { MeetingRow } from "../../bindings";
import { ipc } from "../../ipc";

export const MEETINGS_KEY = ["meetings"] as const;
const PAGE = 200; // search and paging are phase 11

/** The library rows; refetched when a meeting's state, job or notes change. */
export function useMeetings() {
  const client = useQueryClient();
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void ipc
      .onCoreEvent((env) => {
        const type = env.event.type;
        if (type === "stateChanged" || type === "notesReady" || type === "jobProgress") void client.invalidateQueries({ queryKey: MEETINGS_KEY });
      })
      .then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  }, [client]);
  return useQuery({
    queryKey: MEETINGS_KEY,
    queryFn: async (): Promise<MeetingRow[]> => {
      const r = await ipc.commands.listMeetings(PAGE, 0);
      if (r.status === "error") throw new Error(r.error);
      return r.data;
    },
  });
}
