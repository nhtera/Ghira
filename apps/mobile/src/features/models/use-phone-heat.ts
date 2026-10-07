// SPDX-License-Identifier: Apache-2.0
// The phone's thermal state (0 nominal .. 3 critical), kept current by the
// core's thermal events: notes written on the phone wait at 2 or more.
import { useEffect, useState } from "react";
import { ipc } from "../../ipc";

export function usePhoneHeat(): number {
  const [level, setLevel] = useState(0);
  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    void ipc.commands
      .thermalLevel()
      .then((l) => alive && setLevel(l))
      .catch(() => {});
    void ipc
      .onMobileEvent((e) => {
        if (e.type === "thermal") setLevel(e.level);
      })
      .then((u) => (alive ? (off = u) : u()));
    return () => {
      alive = false;
      off?.();
    };
  }, []);
  return level;
}
