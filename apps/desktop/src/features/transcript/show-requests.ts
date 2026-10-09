// SPDX-License-Identifier: Apache-2.0
// "Show in transcript" for a moment the open transcript may already be at: the
// route's `?t=` does not change then, so a mounted transcript hears it here.
type Listener = (tMs: number) => void;
const listeners = new Set<Listener>();

export const showRequests = {
  emit: (tMs: number) => listeners.forEach((l) => l(tMs)),
  subscribe(l: Listener): () => void {
    listeners.add(l);
    return () => void listeners.delete(l);
  },
};
