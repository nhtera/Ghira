// SPDX-License-Identifier: Apache-2.0
// The phone's view of computer sync (phase 15): the status, the one computer
// it is paired with, and a reload whenever the core says something happened.
// `useSyncAvailable` is whether pairing is offered at all (onboarding state).
import { useCallback, useEffect, useState } from "react";
import type { DeviceRow, SyncEvent, SyncStatus } from "../../bindings";
import { ipc } from "../../ipc";
import { unwrap, useResource } from "../settings/api";

const load = async () => unwrap(await ipc.commands.syncStatus());

/** The paired computer; one at most (a spoke pairs with exactly one hub). */
export const pairedDevice = (status: SyncStatus | undefined): DeviceRow | null =>
  status?.paired.find((d) => d.state === "paired") ?? null;

/** Runs `on` for every sync event while mounted. */
export function useSyncEvents(on: (e: SyncEvent) => void) {
  useEffect(() => {
    let alive = true;
    let stop: (() => void) | undefined;
    void ipc.onSyncEvent((e) => alive && on(e)).then(
      (un) => (alive ? (stop = un) : un()),
      () => undefined,
    );
    return () => {
      alive = false;
      stop?.();
    };
  }, [on]);
}

export function useSync() {
  const res = useResource(load);
  const { reload } = res;
  useSyncEvents(useCallback(() => reload(), [reload]));
  return {
    status: res.data,
    /** The computer a Desktop target would go to; null until paired. */
    device: pairedDevice(res.data),
    /** A computer that was told to wipe and has not answered yet. */
    wiping: res.data?.paired.find((d) => d.state === "wipePending") ?? null,
    loadError: res.error,
    reload,
  };
}

/** Pairing is offered (the core's `syncAvailable`); false until known. */
export function useSyncAvailable(): boolean {
  const [available, setAvailable] = useState(false);
  useEffect(() => {
    let alive = true;
    ipc.commands.onboardingState().then(
      (r) => alive && setAvailable(r.status === "ok" && r.data.syncAvailable),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, []);
  return available;
}
