// SPDX-License-Identifier: Apache-2.0
// Where the app lock stands, shared by the root view (no routes while locked),
// the gate and the sheet hosts. "unknown" until Rust answered `lock_state`; it
// answers an error while the app is starting, so the call is retried with a
// growing pause. `covered` hides the screen as soon as the page is hidden when
// the app lock is on (the app switcher snapshot), before Rust has locked.
import { useSyncExternalStore } from "react";
import { ipc } from "../../ipc";
import { LOCKED_EVENT, UNLOCKED_EVENT } from "./events";

export type LockPhase = "unknown" | "locked" | "unlocked";
type Snapshot = { phase: LockPhase; covered: boolean };

const FIRST_RETRY_MS = 400;
const MAX_RETRY_MS = 4000;

let snap: Snapshot = { phase: "unknown", covered: false };
let lockEnabled = false;
let retryMs = FIRST_RETRY_MS;
let timer: ReturnType<typeof setTimeout> | undefined;
const listeners = new Set<() => void>();

function set(next: Partial<Snapshot>) {
  const prev = snap;
  snap = { ...snap, ...next };
  if (snap.phase === prev.phase && snap.covered === prev.covered) return;
  if (snap.phase !== prev.phase) {
    if (snap.phase === "locked") window.dispatchEvent(new Event(LOCKED_EVENT));
    if (snap.phase === "unlocked" && prev.phase === "locked")
      window.dispatchEvent(new Event(UNLOCKED_EVENT));
  }
  listeners.forEach((l) => l());
}

/** Asks Rust; on an error ("the app is starting") asks again later. */
export async function refreshLock(): Promise<void> {
  clearTimeout(timer);
  try {
    const r = await ipc.commands.lockState();
    if (r.status === "ok") {
      retryMs = FIRST_RETRY_MS;
      // Keep the cover until the page is visible again.
      set({
        phase: r.data ? "locked" : "unlocked",
        covered: document.visibilityState === "hidden" && lockEnabled,
      });
      void readLockEnabled();
      return;
    }
  } catch {
    /* retried below */
  }
  timer = setTimeout(() => void refreshLock(), retryMs);
  retryMs = Math.min(retryMs * 2, MAX_RETRY_MS);
}

async function readLockEnabled() {
  const s = await ipc.commands.getSettings().catch(() => null);
  if (s?.status === "ok") lockEnabled = s.data.appLock;
}

/** Settings tells the store when the user turns the lock on or off. */
export function setLockEnabled(on: boolean) {
  lockEnabled = on;
}

/** The page went to the background or came back. */
export function pageVisibility(visible: boolean) {
  if (!visible) {
    if (lockEnabled) set({ covered: true });
    return;
  }
  set({ covered: false });
  void refreshLock();
}

/** The core says the lock engaged or was released (`lock-changed`). */
export function applyLockEvent(locked: boolean) {
  if (locked) set({ phase: "locked" });
  else set({ phase: "unlocked", covered: false });
}

/** The user unlocked through the gate. */
export function markUnlocked() {
  set({ phase: "unlocked", covered: false });
}

export const subscribeLock = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};
export const getLockSnapshot = () => snap;

export function useLock(): Snapshot {
  return useSyncExternalStore(subscribeLock, getLockSnapshot);
}

/** For tests: back to "not asked yet". */
export function resetLockStore() {
  clearTimeout(timer);
  snap = { phase: "unknown", covered: false };
  lockEnabled = false;
  retryMs = FIRST_RETRY_MS;
}
