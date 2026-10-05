// SPDX-License-Identifier: Apache-2.0
// The mock core's computer sync (phase 15, slice 15-B): the phone side of
// doc 07 as state over the real DTOs. Starts off; `?sync=<state>` or
// `__ghiMock.syncSet(state)` jumps to one of:
//   off            not paired, sync off
//   pairing        on, not paired; the camera scan is waiting for a code
//   paired         paired with a MacBook Pro
//   wipePending    the computer asked this phone to wipe (shown until WipeDone)
//   conflict       paired, and the meeting has a conflict copy
//   needsConfirm   paired, the computer deleted 12 meetings and waits for an answer
//   error          paired, the last session failed (code "unreachable"), 2 items waiting
// Other hooks: syncSimulateScan (a good scan), syncSimulateWipeDone, syncSetDeleteEverywhere.
import type { ConflictCopy, DeleteEverywhereStatus, DeviceRow, SyncEvent, SyncStatus } from "../bindings";
import type { Commands } from "./ipc";

const ok = <T>(data: T) => ({ status: "ok" as const, data });
const fail = (error: string) => ({ status: "error" as const, error });

export type MockSyncState = "off" | "pairing" | "paired" | "wipePending" | "conflict" | "needsConfirm" | "error";
const STATES: readonly MockSyncState[] = ["off", "pairing", "paired", "wipePending", "conflict", "needsConfirm", "error"];

const flag = () => new URLSearchParams(location.search).get("sync");

export const MOCK_DESKTOP: DeviceRow = {
  gid: "device-macbook",
  name: "MacBook Pro",
  platform: "mac",
  state: "paired",
  lastSeenMs: null,
};

export const MOCK_CONFLICT: ConflictCopy = {
  gid: "conflict-1",
  targetKind: "segment",
  field: "text",
  device: "MacBook Pro",
  text: "Chốt scope cho bản beta vào thứ Sáu.",
};

type SyncCommands = Pick<
  Commands,
  | "syncStatus"
  | "syncSetEnabled"
  | "syncDevices"
  | "syncUnpair"
  | "syncUnpairAndWipe"
  | "syncNow"
  | "syncConflicts"
  | "syncConflictResolve"
  | "syncConfirmMassDelete"
  | "syncDeleteEverywhereStatus"
  | "syncPairScanStart"
  | "syncPairScanStop"
  | "syncLeaseRevoke"
>;

export interface SyncMock {
  commands: SyncCommands;
  listeners: Set<(e: SyncEvent) => void>;
  hooks: {
    syncSet(state: MockSyncState): void;
    syncSimulateScan(): void;
    syncSimulateWipeDone(): void;
    syncSetDeleteEverywhere(state: DeleteEverywhereStatus["state"], waitingFor?: string[]): void;
  };
  /** Meetings whose final pass the phone took back (tests read it). */
  revoked: string[];
}

export function createSyncMock(): SyncMock {
  const listeners = new Set<(e: SyncEvent) => void>();
  const emit = (e: SyncEvent) => listeners.forEach((l) => l(e));
  const revoked: string[] = [];

  let state: MockSyncState = "off";
  let devices: DeviceRow[] = [];
  let conflicts: ConflictCopy[] = [];
  let lastErrorCode: string | null = null;
  let scanning = false;
  let deleteStatus: DeleteEverywhereStatus = { state: "idle", waitingFor: [] };

  const desktop = (over: Partial<DeviceRow> = {}): DeviceRow => ({ ...MOCK_DESKTOP, lastSeenMs: Date.now() - 4 * 60_000, ...over });

  const set = (next: MockSyncState) => {
    state = next;
    scanning = next === "pairing";
    devices = next === "off" || next === "pairing" ? [] : [desktop(next === "wipePending" ? { state: "wipePending" } : {})];
    conflicts = next === "conflict" ? [{ ...MOCK_CONFLICT }] : [];
    lastErrorCode = next === "error" ? "unreachable" : null;
    if (next === "conflict") emit({ type: "conflict", meeting: "m1" });
    if (next === "needsConfirm") emit({ type: "needsConfirm", device: MOCK_DESKTOP.name, count: 12 });
    if (next === "error") emit({ type: "error", code: "unreachable" });
  };
  const initial = flag();
  if (initial && (STATES as readonly string[]).includes(initial)) set(initial as MockSyncState);

  const status = (): SyncStatus => ({
    enabled: state !== "off",
    paired: devices,
    pendingOnPhone: state === "error" ? 2 : 0,
    localOnly: true,
    lastErrorCode,
  });

  const commands: SyncCommands = {
    syncStatus: async () => ok(status()),
    syncSetEnabled: async (enabled) => {
      if (enabled && state === "off") set("pairing");
      if (!enabled) set("off");
      return ok(status());
    },
    syncDevices: async () => ok(devices),
    syncUnpair: async (gid) => {
      const d = devices.find((x) => x.gid === gid);
      if (!d) return fail("unknown_device");
      devices = devices.filter((x) => x !== d);
      set("pairing");
      emit({ type: "unpaired", gid, name: d.name, byPeer: false });
      return ok(null);
    },
    syncUnpairAndWipe: async (gid) => {
      const d = devices.find((x) => x.gid === gid);
      if (!d) return fail("unknown_device");
      devices = devices.map((x) => (x === d ? { ...x, state: "wipePending" } : x));
      state = "wipePending";
      return ok(null);
    },
    syncNow: async () => {
      if (state === "off") return fail("sync_off");
      if (state === "error") {
        state = "paired";
        lastErrorCode = null;
      }
      emit({ type: "progress", pending: 0 });
      return ok(null);
    },
    syncConflicts: async () => ok(conflicts),
    syncConflictResolve: async (gid) => {
      if (!conflicts.some((c) => c.gid === gid)) return fail("unknown_conflict");
      conflicts = conflicts.filter((c) => c.gid !== gid);
      if (conflicts.length === 0 && state === "conflict") state = "paired";
      return ok(null);
    },
    syncConfirmMassDelete: async () => {
      if (state !== "needsConfirm") return fail("nothing_to_confirm");
      state = "paired";
      return ok(null);
    },
    syncDeleteEverywhereStatus: async () => ok(deleteStatus),
    syncPairScanStart: async () => {
      if (state === "off") return fail("sync_off");
      scanning = true;
      return ok(null);
    },
    syncPairScanStop: async () => {
      scanning = false;
      return ok(null);
    },
    syncLeaseRevoke: async (meeting) => {
      revoked.push(meeting);
      return ok(null);
    },
  };

  return {
    commands,
    listeners,
    revoked,
    hooks: {
      syncSet: set,
      syncSimulateScan: () => {
        if (!scanning) return;
        scanning = false;
        const d = desktop();
        devices = [d];
        state = "paired";
        emit({ type: "paired", device: d });
      },
      syncSimulateWipeDone: () => {
        const d = devices.find((x) => x.state === "wipePending");
        if (!d) return;
        devices = devices.filter((x) => x !== d);
        state = "pairing";
        emit({ type: "wipeDone", gid: d.gid });
      },
      syncSetDeleteEverywhere: (s, waitingFor = []) => {
        deleteStatus = { state: s, waitingFor };
      },
    },
  };
}
