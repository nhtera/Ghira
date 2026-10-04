// SPDX-License-Identifier: Apache-2.0
// Pure: the audio bar's bars. Loudness per 100 ms (0..255, from waveform_peaks)
// is folded into `count` bars; each bar takes the colour slot of whoever spoke
// at its midpoint (0: nobody, drawn neutral).
import type { MeetingSpeaker, SegmentView, Waveform } from "../../bindings";

export type WaveBar = { height: number; slot: number };

/** The shortest bar, as a fraction of the height: silence is still drawn. */
export const MIN_BAR = 0.12;
/** The bars drawn when the waveform is missing or not here yet. */
const PLACEHOLDER = 0.28;

export function waveBars(
  wave: Pick<Waveform, "perSecond" | "peaks"> | null,
  durationMs: number | null,
  segments: readonly Pick<SegmentView, "speakerGid" | "t0Ms" | "t1Ms">[],
  speakers: readonly Pick<MeetingSpeaker, "gid" | "colorSlot">[],
  count: number,
): WaveBar[] {
  const peaks = wave?.peaks ?? [];
  const span = peaks.length > 0 && wave ? (peaks.length / wave.perSecond) * 1000 : (durationMs ?? 0);
  const slotOf = new Map(speakers.map((s) => [s.gid, s.colorSlot]));
  return Array.from({ length: count }, (_, i) => {
    const t0 = span > 0 ? (i / count) * span : 0;
    const t1 = span > 0 ? ((i + 1) / count) * span : 0;
    const mid = (t0 + t1) / 2;
    const seg = segments.find((s) => s.t0Ms !== null && s.t1Ms !== null && s.t0Ms <= mid && mid < s.t1Ms);
    const slot = seg?.speakerGid ? (slotOf.get(seg.speakerGid) ?? 0) : 0;
    if (peaks.length === 0 || !wave) return { height: PLACEHOLDER, slot };
    const from = Math.floor((t0 / 1000) * wave.perSecond);
    const to = Math.max(from + 1, Math.ceil((t1 / 1000) * wave.perSecond));
    let peak = 0;
    for (let k = from; k < to && k < peaks.length; k += 1) peak = Math.max(peak, peaks[k]);
    return { height: Math.max(MIN_BAR, peak / 255), slot };
  });
}
