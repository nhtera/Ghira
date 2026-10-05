// SPDX-License-Identifier: Apache-2.0
// How the rest of the app talks to the (globally mounted) import inbox: the
// number of files waiting, for the Settings row, and a request to open the
// review sheet from anywhere (the banner is not the only door).
import { useSyncExternalStore } from "react";

const OPEN_EVENT = "ghi:open-inbox";
let waiting = 0;
const listeners = new Set<() => void>();

/** The inbox reports how many files wait (0 while locked). */
export function setWaitingCount(n: number): void {
  if (n === waiting) return;
  waiting = n;
  listeners.forEach((l) => l());
}

export function useWaitingCount(): number {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => waiting,
  );
}

/** Opens the review sheet (nothing happens while no file waits). */
export function openInbox(): void {
  window.dispatchEvent(new Event(OPEN_EVENT));
}

export const OPEN_INBOX_EVENT = OPEN_EVENT;

// Where the banner and the "Added" toast are drawn: a slot at the top of the
// tab shell, in the page flow, so they push the screen down instead of
// covering its title (the shell registers the element).
let slot: HTMLElement | null = null;

export function setNoticeSlot(el: HTMLElement | null): void {
  if (el === slot) return;
  slot = el;
  listeners.forEach((l) => l());
}

export function useNoticeSlot(): HTMLElement | null {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => slot,
  );
}
