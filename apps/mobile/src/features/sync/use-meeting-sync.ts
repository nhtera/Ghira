// SPDX-License-Identifier: Apache-2.0
// What sync means for one open meeting (M4): whether a final pass is open on
// the computer (the transcript is read-only), whether its audio lives there
// (both are the core's `leaseOpen` and `audioOnPeer`), a conflict copy waiting
// for a choice, and taking the job back.
import { useCallback, useEffect, useState } from "react";
import type { ConflictCopy, MeetingDetail } from "../../bindings";
import { ipc } from "../../ipc";
import { unwrap, useAction } from "../settings/api";
import { useSync, useSyncEvents } from "./use-sync";

export function useMeetingSync(meeting: string, detail: MeetingDetail | undefined, reload: () => void) {
  const sync = useSync();
  const [conflicts, setConflicts] = useState<ConflictCopy[]>([]);
  const action = useAction();
  const [tick, setTick] = useState(0);

  useEffect(() => {
    let alive = true;
    ipc.commands.syncConflicts(meeting).then(
      (r) => alive && setConflicts(r.status === "ok" ? r.data : []),
      () => alive && setConflicts([]),
    );
    return () => {
      alive = false;
    };
  }, [meeting, tick]);
  useSyncEvents(
    useCallback(
      (e) => {
        if (e.type === "conflict" && e.meeting === meeting) setTick((n) => n + 1);
        if (e.type === "progress") reload();
      },
      [meeting, reload],
    ),
  );

  const paired = sync.device !== null;
  // A final pass open on the computer: the phone reads, the computer writes.
  const lease = paired ? (detail?.leaseOpen ?? null) : null;
  // The audio is on the computer: it recorded the meeting and none is kept here.
  const audioOnDevice = paired && detail?.audioOnPeer === true;

  return {
    device: lease?.device ?? sync.device?.name ?? null,
    leaseOpen: lease !== null,
    /** How far the computer's final pass is, 0..100 (null: none open). */
    leasePercent: lease?.percent ?? null,
    audioOnDevice,
    conflict: conflicts[0] ?? null,
    busy: action.busy,
    error: action.error,
    clearError: action.clear,
    /** Takes the final pass back from the computer. Resolves true when it was taken. */
    async processHere() {
      const done = await action.run(async () => {
        unwrap(await ipc.commands.syncLeaseRevoke(meeting));
        return true;
      });
      if (done) reload();
      return done === true;
    },
    /** Uses the other device's text (`useIt`) or dismisses the copy. */
    async resolve(useIt: boolean) {
      const c = conflicts[0];
      if (!c) return;
      const done = await action.run(async () => {
        unwrap(await ipc.commands.syncConflictResolve(c.gid, useIt));
        return true;
      });
      if (done) {
        setTick((n) => n + 1);
        if (useIt) reload();
      }
    },
  };
}
