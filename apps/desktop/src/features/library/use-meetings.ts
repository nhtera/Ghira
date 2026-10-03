// SPDX-License-Identifier: Apache-2.0
import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo } from "react";
import type { MeetingRow } from "../../bindings";
import { ipc } from "../../ipc";

export const MEETINGS_KEY = ["meetings"] as const;
const PAGE = 200; // the command allows 500; a small first page paints fast
const MAX_ROWS = 2000;

/** The library rows (newest first), loaded page by page up to MAX_ROWS; refetched when a meeting's state or notes change or a job starts or ends. */
export function useMeetings() {
  const client = useQueryClient();
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void ipc
      .onCoreEvent((env) => {
        const e = env.event;
        // Progress ticks only move the row's % (the processing store has it); a job starting or finishing changes the row.
        const edge = e.type === "jobProgress" && e.stage === null && (e.progress == null || e.progress <= 0 || e.progress >= 1);
        if (e.type === "stateChanged" || e.type === "notesReady" || edge) void client.invalidateQueries({ queryKey: MEETINGS_KEY });
      })
      .then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  }, [client]);
  const q = useInfiniteQuery({
    queryKey: MEETINGS_KEY,
    initialPageParam: 0,
    queryFn: async ({ pageParam }): Promise<MeetingRow[]> => {
      const r = await ipc.commands.listMeetings(PAGE, pageParam);
      if (r.status === "error") throw new Error(r.error);
      return r.data;
    },
    getNextPageParam: (last, all) => (last.length < PAGE || all.length * PAGE >= MAX_ROWS ? undefined : all.length * PAGE),
  });
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = q;
  useEffect(() => {
    if (hasNextPage && !isFetchingNextPage) void fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);
  const rows = useMemo(() => q.data?.pages.flat() ?? [], [q.data]);
  return {
    rows,
    isSuccess: q.isSuccess,
    refetch: q.refetch,
    loadingMore: hasNextPage,
  };
}
