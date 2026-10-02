// SPDX-License-Identifier: Apache-2.0
// Pure helpers of the audio bar's waveform: reducing the 100 ms loudness
// buckets to one value per drawn column, and who speaks in each column.
import type { MeetingSpeaker, SegmentView } from "../../bindings";
import { segmentAt } from "../transcript/logic";

/** Peak (0..255) of each of `cols` equal slices of the meeting. */
export function reducePeaks(peaks: readonly number[], perSecond: number, durationMs: number, cols: number): Uint8Array {
  const out = new Uint8Array(Math.max(0, cols));
  if (!cols || !durationMs || !peaks.length || perSecond <= 0) return out;
  const perMs = perSecond / 1000;
  for (let c = 0; c < cols; c++) {
    const from = Math.floor(((c * durationMs) / cols) * perMs);
    const to = Math.max(from + 1, Math.ceil((((c + 1) * durationMs) / cols) * perMs));
    let max = 0;
    for (let i = from; i < to && i < peaks.length; i++) if (peaks[i]! > max) max = peaks[i]!;
    out[c] = max;
  }
  return out;
}

/** Speaker color slot (1..8; 0 = nobody known) of each column's middle, from the transcript lines. */
export function columnSlots(segments: readonly SegmentView[], speakers: readonly MeetingSpeaker[], durationMs: number, cols: number): Uint8Array {
  const out = new Uint8Array(Math.max(0, cols));
  if (!cols || !durationMs) return out;
  const slotOf = new Map(speakers.map((s) => [s.gid, s.colorSlot]));
  for (let c = 0; c < cols; c++) {
    const at = segmentAt(segments, ((c + 0.5) * durationMs) / cols, 0);
    const gid = at >= 0 ? segments[at]!.speakerGid : null;
    out[c] = (gid && slotOf.get(gid)) || 0;
  }
  return out;
}

/** Time under a pointer at `x` of `width` px. */
export const timeAt = (x: number, width: number, durationMs: number) => (width > 0 ? Math.min(durationMs, Math.max(0, (x / width) * durationMs)) : 0);
