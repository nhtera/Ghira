// SPDX-License-Identifier: Apache-2.0
// The mock core's phone sync (phase 15, slice 15-B): the desktop side of
// doc 07 as state over the real DTOs. Starts off; `?sync=<state>` or
// `__ghiMock.syncSet(state)` jumps to one of:
//   off            sync is off
//   pairing        on, nobody paired; syncPairOpen returns a QR placeholder (120 s)
//   paired         one phone paired and synced
//   wipePending    "Unpair and wipe" chosen, waiting for the phone
//   conflict       paired, and the meeting has a conflict copy
//   needsConfirm   paired, the phone deleted 12 meetings and waits for an answer
//   error          paired, the last session failed (code "unreachable")
// Other hooks: syncSimulatePaired, syncSimulateWipeDone, syncSetDeleteEverywhere.
import type { ConflictCopy, DeleteEverywhereStatus, DeviceRow, SyncEvent, SyncStatus } from "../bindings";
import type { Commands } from "./ipc";

type Result<T> = { status: "ok"; data: T } | { status: "error"; error: string };
const ok = <T>(data: T): Promise<Result<T>> => Promise.resolve({ status: "ok", data });
const fail = <T>(error: string): Promise<Result<T>> => Promise.resolve({ status: "error", error });

export type MockSyncState = "off" | "pairing" | "paired" | "wipePending" | "conflict" | "needsConfirm" | "error";
const STATES: readonly MockSyncState[] = ["off", "pairing", "paired", "wipePending", "conflict", "needsConfirm", "error"];

/** How long a pairing code works (the real core: `PAIR_CODE_TTL_MS`). */
export const PAIR_TTL_MS = 120_000;

/** Stands in for the QR code the core renders; the UI only displays the SVG. */
const QR_PLACEHOLDER_SVG =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 29 29" role="img" aria-label="QR code">' +
  '<rect width="29" height="29" fill="#fff"/>' +
  '<path fill="#000" d="M2 2h7v7H2zM20 2h7v7h-7zM2 20h7v7H2zM11 3h2v2h-2zM14 6h3v2h-3zM11 11h4v2h-4zM17 11h3v4h-3zM11 16h2v4h-2zM15 18h4v3h-4zM21 14h4v2h-4zM20 20h2v5h-2zM24 22h3v3h-3z"/>' +
  "</svg>";

const flag = () => new URLSearchParams(location.search).get("sync");

export const MOCK_PHONE: DeviceRow = {
  gid: "device-iphone",
  name: "iPhone 16",
  platform: "ios",
  state: "paired",
  lastSeenMs: null,
};

export const MOCK_CONFLICT: ConflictCopy = {
  gid: "conflict-1",
  targetKind: "segment",
  field: "text",
  device: "iPhone 16",
  text: "Chốt scope cho bản beta vào thứ Sáu.",
};

type SyncCommands = Pick<
  Commands,
  | "syncStatus"
  | "syncSetEnabled"
  | "syncPairOpen"
  | "syncPairClose"
  | "syncDevices"
  | "syncUnpair"
  | "syncUnpairAndWipe"
  | "syncNow"
  | "syncConflicts"
  | "syncConflictResolve"
  | "syncConfirmMassDelete"
  | "syncDeleteEverywhereStatus"
>;

export interface SyncMock {
  commands: SyncCommands;
  listeners: Set<(e: SyncEvent) => void>;
  hooks: {
    syncSet(state: MockSyncState): void;
    syncSimulatePaired(): void;
    syncSimulateWipeDone(): void;
    syncSetDeleteEverywhere(state: DeleteEverywhereStatus["state"], waitingFor?: string[]): void;
  };
}

export function createSyncMock(): SyncMock {
  const listeners = new Set<(e: SyncEvent) => void>();
  const emit = (e: SyncEvent) => listeners.forEach((l) => l(e));

  let state: MockSyncState = "off";
  let devices: DeviceRow[] = [];
  let conflicts: ConflictCopy[] = [];
  let lastErrorCode: string | null = null;
  let deleteStatus: DeleteEverywhereStatus = { state: "idle", waitingFor: [] };

  const phone = (over: Partial<DeviceRow> = {}): DeviceRow => ({ ...MOCK_PHONE, lastSeenMs: Date.now() - 4 * 60_000, ...over });

  const set = (next: MockSyncState) => {
    state = next;
    devices = next === "off" || next === "pairing" ? [] : [phone(next === "wipePending" ? { state: "wipePending" } : {})];
    conflicts = next === "conflict" ? [{ ...MOCK_CONFLICT }] : [];
    lastErrorCode = next === "error" ? "unreachable" : null;
    if (next === "conflict") emit({ type: "conflict", meeting: "m1" });
    if (next === "needsConfirm") emit({ type: "needsConfirm", device: MOCK_PHONE.name, count: 12 });
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
  const needsOn = <T>(): Promise<Result<T>> | null => (state === "off" ? fail<T>("sync_off") : null);
  const deviceOf = (gid: string) => devices.find((d) => d.gid === gid);

  const commands: SyncCommands = {
    syncStatus: () => ok(status()),
    syncSetEnabled: (enabled) => {
      if (enabled && state === "off") set("pairing");
      if (!enabled) set("off");
      return ok(status());
    },
    syncPairOpen: () => needsOn() ?? ok({ qrSvg: QR_PLACEHOLDER_SVG, expiresMs: PAIR_TTL_MS }),
    syncPairClose: () => ok(null),
    syncDevices: () => ok(devices),
    syncUnpair: (gid) => {
      const d = deviceOf(gid);
      if (!d) return fail("unknown_device");
      devices = devices.filter((x) => x !== d);
      if (devices.length === 0) set("pairing");
      emit({ type: "unpaired", gid, name: d.name, byPeer: false });
      return ok(null);
    },
    syncUnpairAndWipe: (gid) => {
      const d = deviceOf(gid);
      if (!d) return fail("unknown_device");
      devices = devices.map((x) => (x === d ? { ...x, state: "wipePending" } : x));
      state = "wipePending";
      return ok(null);
    },
    syncNow: () => {
      const off = needsOn<null>();
      if (off) return off;
      if (state === "error") {
        state = "paired";
        lastErrorCode = null;
      }
      emit({ type: "progress", pending: 0 });
      return ok(null);
    },
    syncConflicts: () => ok(conflicts),
    syncConflictResolve: (gid) => {
      if (!conflicts.some((c) => c.gid === gid)) return fail("unknown_conflict");
      conflicts = conflicts.filter((c) => c.gid !== gid);
      if (conflicts.length === 0 && state === "conflict") state = "paired";
      return ok(null);
    },
    syncConfirmMassDelete: () => {
      if (state !== "needsConfirm") return fail("nothing_to_confirm");
      state = "paired";
      return ok(null);
    },
    syncDeleteEverywhereStatus: () => ok(deleteStatus),
  };

  return {
    commands,
    listeners,
    hooks: {
      syncSet: set,
      syncSimulatePaired: () => {
        const d = phone();
        devices = [d];
        state = "paired";
        emit({ type: "paired", device: d });
      },
      syncSimulateWipeDone: () => {
        const d = devices.find((x) => x.state === "wipePending");
        if (!d) return;
        devices = devices.filter((x) => x !== d);
        state = devices.length === 0 ? "pairing" : "paired";
        emit({ type: "wipeDone", gid: d.gid });
      },
      syncSetDeleteEverywhere: (s, waitingFor = []) => {
        deleteStatus = { state: s, waitingFor };
      },
    },
  };
}
