// SPDX-License-Identifier: Apache-2.0
import { useEffect, useState } from "react";
import { ipc } from "../../ipc";

/**
 * Whether the user chose to offer cloud notes (Settings -> Cloud notes). The
 * core refuses every cloud call until they do, so the cloud entry points stay
 * hidden while this is false (or not known yet).
 */
export function useCloudOffered(): boolean {
  const [offered, setOffered] = useState(false);
  useEffect(() => {
    let alive = true;
    ipc.commands.getSettings().then(
      (r) => alive && r.status === "ok" && setOffered(r.data.cloudOffered),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, []);
  return offered;
}
