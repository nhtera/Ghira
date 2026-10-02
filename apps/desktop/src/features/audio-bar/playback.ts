// SPDX-License-Identifier: Apache-2.0
// The audio bar's side of the player store (state/player.ts), apart from the
// DOM so it tests against a fake element.
import { nextSpeech, type SeekRequest } from "../../state/player";

export type AudioLike = Pick<HTMLAudioElement, "currentTime" | "pause"> & { play(): Promise<void> | void };

/**
 * One time report: where the element is (meeting ms). With `skipSilence` on,
 * a gap between speech longer than 1.5 s is jumped over first.
 */
export function readTime(audio: Pick<AudioLike, "currentTime">, spans: readonly { t0Ms: number; t1Ms: number }[], skipSilence: boolean): number {
  let ms = audio.currentTime * 1000;
  if (skipSilence) {
    const to = nextSpeech(spans, ms);
    if (to != null) {
      audio.currentTime = to / 1000;
      ms = to;
    }
  }
  return ms;
}

/** Carries out a seek / play / pause request; a refused play() is reported through `onRefused`. */
export function carryOut(audio: AudioLike, req: SeekRequest, onRefused: () => void) {
  audio.currentTime = req.ms / 1000;
  if (!req.play) return audio.pause();
  try {
    void Promise.resolve(audio.play()).catch(onRefused);
  } catch {
    onRefused();
  }
}
