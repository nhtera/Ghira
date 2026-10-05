// SPDX-License-Identifier: Apache-2.0
// Where the app lock stands, shared by the root view (no routes while locked),
// the gate and the sheet hosts. "unknown" until Rust answered `lock_state`; it
// answers an error while the app is starting, so the call is retried with a
// growing pause. When it keeps failing, `store_status` says why: "unavailable"
// (the encrypted store can't be opened: a code in `problem`) or, for anything
// else, "failed" after a few tries. Both are screens with a way forward, never
// a blank page. `covered` hides the screen as soon as the page is hidden when
// the app lock is on (the app switcher snapshot), before Rust has locked.
import { useSyncExternalStore } from "react";
import type { StoreProblem } from "../../bindings";
import { ipc } from "../../ipc";
import { LOCKED_EVENT, UNLOCKED_EVENT } from "./events";

export type LockPhase = "unknown" | "locked" | "unlocked" | "unavailable" | "failed";
type Snapshot = { phase: LockPhase; covered: boolean; problem: StoreProblem | null };

const FIRST_RETRY_MS = 400;
const MAX_RETRY_MS = 4000;
/** Failed answers (about ten seconds) before "failed" shows instead of nothing. */
const FAILURES_BEFORE_SCREEN = 5;

let snap: Snapshot = { phase: "unknown", covered: false, problem: null };
let failures = 0;
let lockEnabled = false;
let retryMs = FIRST_RETRY_MS;
let timer: ReturnType<typeof setTimeout> | undefined;
const listeners = new Set<() => void>();

function set(next: Partial<Snapshot>) {
  const prev = snap;
  snap = { ...snap, ...next };
  if (snap.phase === prev.phase && snap.covered === prev.covered && snap.problem === prev.problem) return;
  if (snap.phase !== prev.phase) {
    if (snap.phase === "locked") window.dispatchEvent(new Event(LOCKED_EVENT));
    if (snap.phase === "unlocked" && prev.phase === "locked")
      window.dispatchEvent(new Event(UNLOCKED_EVENT));
  }
  listeners.forEach((l) => l());
}

/**
 * Asks Rust; on an error ("the app is starting") asks again later. While the
 * store can't be opened it does nothing: every ask opens the store again (and
 * may prompt for its key), so only "Try again" does.
 */
export async function refreshLock(): Promise<void> {
  if (snap.phase === "unavailable") return;
  await ask();
}

async function ask(): Promise<void> {
  clearTimeout(timer);
  try {
    const r = await ipc.commands.lockState();
    if (r.status === "ok") {
      retryMs = FIRST_RETRY_MS;
      failures = 0;
      // Keep the cover until the page is visible again.
      set({
        phase: r.data ? "locked" : "unlocked",
        covered: document.visibilityState === "hidden" && lockEnabled,
        problem: null,
      });
      void readLockEnabled();
      return;
    }
  } catch {
    /* looked into below */
  }
  if (await storeProblem()) return;
  failures += 1;
  if (failures >= FAILURES_BEFORE_SCREEN && snap.phase !== "unlocked") {
    set({ phase: "failed", problem: null });
  }
  timer = setTimeout(() => void refreshLock(), retryMs);
  retryMs = Math.min(retryMs * 2, MAX_RETRY_MS);
}

/** The store can't be opened: show why (the core remembers it; nothing opens again), and stop asking until the user retries. */
async function storeProblem(): Promise<boolean> {
  try {
    const r = await ipc.commands.storeStatus();
    if (r.status === "ok" && r.data.state === "unavailable") {
      // The store opened but starting on it failed: the generic screen, no way to erase.
      if (r.data.problem === "startup") set({ phase: "failed", problem: null });
      else set({ phase: "unavailable", problem: r.data.problem });
      return true;
    }
  } catch {
    /* not a store problem we can name */
  }
  return false;
}

/** "Try again" on the can't-open and couldn't-load screens. */
export async function retryStartup(): Promise<void> {
  failures = 0;
  retryMs = FIRST_RETRY_MS;
  await ask();
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
  snap = { phase: "unknown", covered: false, problem: null };
  failures = 0;
  lockEnabled = false;
  retryMs = FIRST_RETRY_MS;
}
