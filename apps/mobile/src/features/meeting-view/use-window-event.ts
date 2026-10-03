// SPDX-License-Identifier: Apache-2.0
import { useEffect, useRef } from "react";

/** Runs `cb` whenever `name` fires on `window`. */
export function useWindowEvent(name: string, cb: () => void) {
  const latest = useRef(cb);
  useEffect(() => {
    latest.current = cb;
  });
  useEffect(() => {
    const on = () => latest.current();
    window.addEventListener(name, on);
    return () => window.removeEventListener(name, on);
  }, [name]);
}
