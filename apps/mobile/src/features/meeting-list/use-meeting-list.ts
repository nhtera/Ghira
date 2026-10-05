// SPDX-License-Identifier: Apache-2.0
// The meetings list's data: pages of rows, a status chip per row (the
// meeting_chips command), and live "Processing on phone %" from coreEvent
// jobProgress. A stateChanged re-reads that row's chip.
import { useCallback, useEffect, useRef, useState } from "react";
import type { CoreEvent, MeetingChip, MeetingRow } from "../../bindings";
import { ipc } from "../../ipc";
import { UNLOCKED_EVENT } from "../app-lock/events";
import { useWindowEvent } from "../meeting-view/use-window-event";
import { classifyError } from "../meeting-view/read-error";
import { useSyncEvents } from "../sync/use-sync";

export const PAGE = 50;

/** The chip with the live percent of its job. A meeting that is not processing yet becomes so on its first progress event. */
export function chipWithProgress(
  chip: MeetingChip | undefined,
  percent: number | undefined,
): MeetingChip | undefined {
  if (percent === undefined) return chip;
  if (
    !chip ||
    chip.kind === "recorded" ||
    chip.kind === "processingOnPhone" ||
    chip.kind === "waitingForModels"
  )
    return { kind: "processingOnPhone", percent };
  return chip;
}

export type MeetingListState = {
  rows: MeetingRow[];
  chips: Record<string, MeetingChip>;
  status: "loading" | "ready" | "error";
  /** The last page came back short. */
  done: boolean;
  refreshing: boolean;
};

const percentOf = (progress: number | null) =>
  Math.round(Math.min(Math.max(progress ?? 0, 0), 1) * 100);

export function useMeetingList() {
  const [state, setState] = useState<MeetingListState>({
    rows: [],
    chips: {},
    status: "loading",
    done: false,
    refreshing: false,
  });
  const [progress, setProgress] = useState<Record<string, number>>({});
  const busy = useRef(false);
  const rowsRef = useRef<MeetingRow[]>([]);
  useEffect(() => {
    rowsRef.current = state.rows;
  }, [state.rows]);

  const fetchChips = useCallback(async (ids: string[]) => {
    if (!ids.length) return;
    const r = await ipc.commands.meetingChips(ids).catch(() => null);
    if (r?.status !== "ok") return;
    setState((s) => ({
      ...s,
      chips: {
        ...s.chips,
        ...Object.fromEntries(r.data.map((c) => [c.gid, c.chip])),
      },
    }));
  }, []);

  const loadRef = useRef<(reset: boolean) => Promise<void>>(async () => {});
  // How many rows the core has handed out: the next page starts there, whatever
  // was deleted from the screen since.
  const fetched = useRef(0);
  // A refresh asked for while one was running runs right after it.
  const queued = useRef(false);

  const load = useCallback(
    async (reset: boolean) => {
      if (busy.current) {
        if (reset) queued.current = true;
        return;
      }
      busy.current = true;
      const offset = reset ? 0 : fetched.current;
      setState((s) =>
        reset && s.rows.length ? { ...s, refreshing: true } : s,
      );
      const r = await ipc.commands.listMeetings(PAGE, offset).catch(() => null);
      busy.current = false;
      if (r?.status !== "ok") {
        // The store is still opening: look again shortly.
        if (r && classifyError(r.error) === "starting")
          window.setTimeout(() => void loadRef.current(true), 1000);
        setState((s) => ({
          ...s,
          status: s.rows.length
            ? "ready"
            : r && classifyError(r.error) === "starting"
              ? "loading"
              : "error",
          refreshing: false,
        }));
      } else {
        const page = r.data;
        fetched.current = reset ? page.length : fetched.current + page.length;
        setState((s) => {
          const have = new Set(reset ? [] : s.rows.map((m) => m.gid));
          return {
            ...s,
            rows: reset
              ? page
              : [...s.rows, ...page.filter((m) => !have.has(m.gid))],
            status: "ready",
            done: page.length < PAGE,
            refreshing: false,
          };
        });
        void fetchChips(page.map((m) => m.gid));
      }
      if (queued.current) {
        queued.current = false;
        void loadRef.current(true);
      }
    },
    [fetchChips],
  );
  useEffect(() => {
    loadRef.current = load;
  }, [load]);

  useEffect(() => {
    void load(true);
  }, [load]);

  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    const onEvent = ({ event: e }: CoreEvent) => {
      if (e.type === "jobProgress" && e.meeting) {
        const id = e.meeting;
        setProgress((p) => ({ ...p, [id]: percentOf(e.progress) }));
      } else if (e.type === "stateChanged") {
        const id = e.meeting;
        if (!rowsRef.current.some((r) => r.gid === id)) {
          void load(true);
          return;
        }
        setProgress((p) =>
          Object.fromEntries(Object.entries(p).filter(([k]) => k !== id)),
        );
        void fetchChips([id]);
      }
    };
    void ipc.onCoreEvent(onEvent).then((u) => {
      if (alive) off = u;
      else u();
    });
    return () => {
      alive = false;
      off?.();
    };
  }, [fetchChips, load]);

  useWindowEvent(UNLOCKED_EVENT, () => void load(true));
  // A pass with the computer moved something (its final pass, a sync): the chips follow.
  useSyncEvents(
    useCallback(
      (e) => {
        if (e.type === "progress") void fetchChips(rowsRef.current.map((r) => r.gid));
      },
      [fetchChips],
    ),
  );

  const chipOf = useCallback(
    (gid: string) => chipWithProgress(state.chips[gid], progress[gid]),
    [state.chips, progress],
  );

  const retry = useCallback(
    async (gid: string) => {
      const r = await ipc.commands.retryMeeting(gid).catch(() => null);
      if (r?.status === "ok") await fetchChips([gid]);
    },
    [fetchChips],
  );

  const [deleteFailed, setDeleteFailed] = useState(false);
  const remove = useCallback(async (gid: string) => {
    const r = await ipc.commands.deleteMeeting(gid).catch(() => null);
    if (r?.status === "ok") {
      fetched.current = Math.max(0, fetched.current - 1);
      setState((s) => ({ ...s, rows: s.rows.filter((m) => m.gid !== gid) }));
    } else setDeleteFailed(true);
  }, []);

  return {
    ...state,
    chipOf,
    loadMore: useCallback(() => load(false), [load]),
    refresh: useCallback(() => load(true), [load]),
    retry,
    remove,
    deleteFailed,
    dismissDeleteFailed: () => setDeleteFailed(false),
  };
}
