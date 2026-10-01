// SPDX-License-Identifier: Apache-2.0
// Compact window (< 1100 px): icons-only sidebar and header (brief §5, §8).
import { useSyncExternalStore } from "react";

const QUERY = "(max-width: 1099px)";
const mq = () => (typeof window !== "undefined" && window.matchMedia ? window.matchMedia(QUERY) : null);

function subscribe(cb: () => void) {
  const m = mq();
  m?.addEventListener("change", cb);
  return () => m?.removeEventListener("change", cb);
}

export function useCompact(): boolean {
  return useSyncExternalStore(subscribe, () => mq()?.matches ?? false, () => false);
}
