// SPDX-License-Identifier: Apache-2.0
// The meeting's audio: a play token from the core (issueAudioPlay) played by
// an <audio> element over the ghi-audio:// scheme. The token is issued on the
// first play, not when the screen opens. One instance per meeting screen.
import { useCallback, useEffect, useRef, useState } from "react";
import { ipc } from "../../ipc";

export const SPEEDS = [1, 1.25, 1.5, 2] as const;

export type AudioState = {
  playing: boolean;
  timeMs: number;
  durationMs: number | null;
  speed: (typeof SPEEDS)[number];
  /** The core refused a token or the file did not play. */
  failed: boolean;
};

export function useAudio(meeting: string, durationHint: number | null) {
  const [state, setState] = useState<AudioState>({
    playing: false,
    timeMs: 0,
    durationMs: null,
    speed: 1,
    failed: false,
  });
  const el = useRef<HTMLAudioElement | null>(null);
  const pending = useRef<Promise<HTMLAudioElement | null> | null>(null);
  const speed = useRef<AudioState["speed"]>(1);

  const patch = useCallback(
    (p: Partial<AudioState>) => setState((s) => ({ ...s, ...p })),
    [],
  );

  // The screen is keyed by the meeting: leaving it stops the audio.
  useEffect(
    () => () => {
      el.current?.pause();
      el.current = null;
      pending.current = null;
    },
    [],
  );

  const ensure = useCallback((): Promise<HTMLAudioElement | null> => {
    if (el.current) return Promise.resolve(el.current);
    patch({ failed: false });
    pending.current ??= (async () => {
      const r = await ipc.commands.issueAudioPlay(meeting).catch(() => null);
      if (r?.status !== "ok") {
        // Forget the failure so the next press asks again.
        pending.current = null;
        patch({ failed: true });
        return null;
      }
      const a = new Audio();
      a.preload = "metadata";
      a.src = ipc.audioUrl(r.data.token);
      a.addEventListener("play", () => patch({ playing: true }));
      a.addEventListener("pause", () => patch({ playing: false }));
      a.addEventListener("ended", () => patch({ playing: false }));
      a.addEventListener("error", () =>
        patch({ playing: false, failed: true }),
      );
      a.addEventListener("timeupdate", () =>
        patch({ timeMs: Math.round(a.currentTime * 1000) }),
      );
      if (r.data.durationMs !== null) patch({ durationMs: r.data.durationMs });
      el.current = a;
      return a;
    })();
    return pending.current;
  }, [meeting, patch]);

  // Smoother than timeupdate (about 4 Hz) for the karaoke highlight.
  useEffect(() => {
    if (!state.playing) return;
    const id = window.setInterval(() => {
      if (el.current)
        patch({ timeMs: Math.round(el.current.currentTime * 1000) });
    }, 120);
    return () => window.clearInterval(id);
  }, [state.playing, patch]);

  const start = useCallback(async (a: HTMLAudioElement) => {
    a.playbackRate = speed.current;
    await a.play().catch(() => undefined);
  }, []);

  const playFrom = useCallback(
    async (ms: number) => {
      const a = await ensure();
      if (!a) return;
      a.currentTime = Math.max(0, ms) / 1000;
      patch({ timeMs: Math.max(0, ms) });
      await start(a);
    },
    [ensure, patch, start],
  );

  const toggle = useCallback(async () => {
    const a = await ensure();
    if (!a) return;
    if (a.paused) await start(a);
    else a.pause();
  }, [ensure, start]);

  const seek = useCallback(
    async (ms: number) => {
      const a = await ensure();
      if (!a) return;
      a.currentTime = ms / 1000;
      patch({ timeMs: ms });
    },
    [ensure, patch],
  );

  const cycleSpeed = useCallback(() => {
    const next = SPEEDS[(SPEEDS.indexOf(speed.current) + 1) % SPEEDS.length];
    speed.current = next;
    if (el.current) el.current.playbackRate = next;
    patch({ speed: next });
  }, [patch]);

  // The token's length wins; until then the meeting's.
  return {
    ...state,
    durationMs: state.durationMs ?? durationHint,
    playFrom,
    toggle,
    seek,
    cycleSpeed,
  };
}
