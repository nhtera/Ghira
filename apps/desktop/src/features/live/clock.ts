// SPDX-License-Identifier: Apache-2.0
// The live meeting clock. Components that only need "the time right now" at an
// event (a note typed, a mark) read it on demand instead of re-rendering.
import { useEffect, useState } from "react";
import { elapsedMs, useLive } from "../../state/live";

/** Meeting time (ms, pauses excluded) at this moment. */
export const meetingMsNow = () => elapsedMs(useLive.getState(), Date.now());

/** Ticks once a second while `active` (clock displays, lanes growing). */
export function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [active]);
  return now;
}
