// SPDX-License-Identifier: Apache-2.0
// What sync means for one open meeting (M4): whether a final pass is open on
// the computer (the transcript is read-only), whether its audio lives there,
// a conflict copy waiting for a choice, and taking the job back.
import { useCallback, useEffect, useState } from "react";
import type { ConflictCopy, MeetingChip, MeetingDetail } from "../../bindings";
import { ipc } from "../../ipc";
import { unwrap, useAction } from "../settings/api";
import { useSync, useSyncEvents } from "./use-sync";

export function useMeetingSync(meeting: string, detail: MeetingDetail | undefined, chip: MeetingChip | undefined, reload: () => void) {
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
  // A final pass open on the computer: the lease is the chip (the phone reads, the computer writes).
  const leaseOpen = paired && chip?.kind === "finalOnDesktop";
  // The audio is on the computer: none here, and a pairing that could hold it. The meeting DTO has
  // no audio-origin field yet, so this is derived from the chip; 15-J can replace it with the real one.
  const audioOnDevice = paired && detail !== undefined && !detail.audioAvailable && (chip?.kind === "finalOnDesktop" || chip?.kind === "synced");

  return {
    device: sync.device?.name ?? null,
    leaseOpen,
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
