// SPDX-License-Identifier: Apache-2.0
// The meeting's waveform (waveform_peaks: computed once by the core, then kept
// sealed with the meeting). Absent until it arrives, and for good if the core
// cannot make it: the audio bar draws still bars then.
import { useEffect, useState } from "react";
import type { Waveform } from "../../bindings";
import { ipc } from "../../ipc";

export function useWaveform(meeting: string, enabled: boolean): Waveform | null {
  const [wave, setWave] = useState<Waveform | null>(null);
  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    void ipc.commands
      .waveformPeaks(meeting)
      .then((r) => alive && r.status === "ok" && setWave(r.data))
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [meeting, enabled]);
  return wave;
}
