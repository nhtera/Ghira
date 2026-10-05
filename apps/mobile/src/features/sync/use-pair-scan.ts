// SPDX-License-Identifier: Apache-2.0
// Pairing from the phone: start the camera scan (native; the code never
// reaches the webview), then follow the core's events. A scan ends in
// `paired`, or in a failure the screen words (scan-error.ts).
import { useCallback, useEffect, useRef, useState } from "react";
import type { DeviceRow, SyncEvent } from "../../bindings";
import { ipc } from "../../ipc";
import { scanFailure, type ScanFailure } from "./scan-error";
import { pairedDevice, useSyncEvents } from "./use-sync";

export type PairPhase = "idle" | "scanning" | "paired" | "failed";

export function usePairScan(autoStart: boolean) {
  const [phase, setPhase] = useState<PairPhase>("idle");
  const [failure, setFailure] = useState<ScanFailure | null>(null);
  const [device, setDevice] = useState<DeviceRow | null>(null);
  const phaseRef = useRef<PairPhase>("idle");
  const to = useCallback((p: PairPhase) => {
    phaseRef.current = p;
    setPhase(p);
  }, []);
  const fail = useCallback(
    (code: string) => {
      setFailure(scanFailure(code));
      to("failed");
    },
    [to],
  );

  const start = useCallback(async () => {
    setFailure(null);
    to("scanning");
    try {
      const on = await ipc.commands.syncSetEnabled(true);
      if (on.status !== "ok") return fail(on.error);
      const r = await ipc.commands.syncPairScanStart();
      if (r.status !== "ok") fail(r.error);
    } catch {
      fail("internal");
    }
  }, [fail, to]);

  useSyncEvents(
    useCallback(
      (e: SyncEvent) => {
        if (e.type === "paired") {
          setDevice(e.device);
          to("paired");
        } else if (e.type === "error" && phaseRef.current === "scanning") fail(e.code);
      },
      [fail, to],
    ),
  );

  // Already paired (back to this step): say so. Otherwise scan at once when asked to.
  useEffect(() => {
    let alive = true;
    ipc.commands.syncStatus().then(
      (r) => {
        if (!alive) return;
        const d = r.status === "ok" ? pairedDevice(r.data) : null;
        if (d) {
          setDevice(d);
          to("paired");
        } else if (autoStart) void start();
      },
      () => alive && autoStart && void start(),
    );
    return () => {
      alive = false;
    };
  }, [autoStart, start, to]);

  // Leaving the screen closes the camera.
  useEffect(
    () => () => {
      if (phaseRef.current === "scanning") void ipc.commands.syncPairScanStop().catch(() => undefined);
    },
    [],
  );

  /** Back to "not scanning" (after an unpair, the old pairing is no longer news). */
  const reset = useCallback(() => {
    setDevice(null);
    setFailure(null);
    to("idle");
  }, [to]);

  return { phase, failure, device, start, reset };
}

export type PairScan = ReturnType<typeof usePairScan>;
