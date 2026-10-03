// SPDX-License-Identifier: Apache-2.0
// Audio bar (D6): the detail screen's only `<audio>`. It carries out the
// requests in state/player.ts (seek, play, pause, stop at a span's end), reports
// the time back (every frame while playing, so the karaoke is smooth), honors
// the speed and skip-silence, and draws the waveform. A failed play token or
// waveform leaves the transcript fully usable.
import { formatClock } from "@ghi/i18n";
import { useCallback, useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Icon, Menu, cn } from "@ghi/ui";
import type { MeetingDetail } from "../../bindings";
import { useMeetingTranscript, useWaveform } from "../../state/meeting-queries";
import { RATES, usePlayer } from "../../state/player";
import { carryOut, readTime } from "./playback";
import { WaveformSlider } from "./waveform";

/** Space plays/pauses unless it would type, or press the focused control. */
const spaceIsOurs = (e: KeyboardEvent) => {
  if (e.key !== " " || e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return false;
  const el = e.target instanceof HTMLElement ? e.target : null;
  if (!el || el === document.body) return true;
  if (el.isContentEditable || el.closest("input, textarea, select, button, a, [role=button], [role=menuitem], [role=dialog], [role=tab]")) return false;
  return true;
};

const playBtn = "grid size-10 shrink-0 place-items-center rounded-full bg-accent text-on-accent hover:brightness-110 disabled:opacity-40 disabled:hover:brightness-100";

export function AudioBar({ meeting, detail }: { meeting: string; detail: MeetingDetail }) {
  const { t } = useTranslation();
  const available = detail.audioAvailable;
  const audio = useRef<HTMLAudioElement>(null);
  const src = usePlayer((s) => s.src);
  const loading = usePlayer((s) => s.loading);
  const error = usePlayer((s) => s.error);
  const playing = usePlayer((s) => s.playing);
  const rate = usePlayer((s) => s.rate);
  const skipSilence = usePlayer((s) => s.skipSilence);
  const tokenMs = usePlayer((s) => s.durationMs);
  const transcript = useMeetingTranscript(meeting);
  const wave = useWaveform(meeting, available);
  const segments = useMemo(() => transcript.data?.segments ?? [], [transcript.data]);
  const spans = useMemo(() => segments.filter((s) => s.t0Ms != null && s.t1Ms != null).map((s) => ({ t0Ms: s.t0Ms!, t1Ms: s.t1Ms! })), [segments]);
  const spansRef = useRef(spans);
  spansRef.current = spans;
  const durationMs = tokenMs || detail.durationMs || 0;

  // A play token expires (10 min) or is revoked by another play: ask for a new one once per source before giving up.
  const retried = useRef<string | null>(null);
  const onError = () => {
    const now = usePlayer.getState().src;
    if (now && retried.current !== now) {
      retried.current = now;
      void usePlayer.getState().reissue();
    } else usePlayer.getState().reportError(t("audio.error"));
  };

  const report = useCallback(() => {
    const el = audio.current;
    if (!el) return;
    const s = usePlayer.getState();
    const ms = readTime(el, spansRef.current, s.skipSilence && s.playing);
    if (ms !== s.currentMs) s.reportTime(ms);
  }, []);

  // Requests from everyone else (a citation chip, a transcript line, the toggle).
  const handled = useRef(0);
  const request = usePlayer((s) => s.seekRequest);
  useEffect(() => {
    const el = audio.current;
    if (!el || !src || !request || request.n === handled.current) return;
    handled.current = request.n;
    carryOut(el, request, () => usePlayer.getState().reportPlaying(false));
  }, [request, src]);

  useEffect(() => {
    const el = audio.current;
    if (!el) return;
    el.playbackRate = rate;
    el.preservesPitch = true;
  }, [rate, src]);

  // Smooth karaoke: every frame while playing.
  useEffect(() => {
    if (!playing) return;
    let id = requestAnimationFrame(function loop() {
      report();
      id = requestAnimationFrame(loop);
    });
    return () => cancelAnimationFrame(id);
  }, [playing, report]);

  useEffect(() => {
    if (!available) return;
    const onKey = (e: KeyboardEvent) => {
      if (!spaceIsOurs(e)) return;
      e.preventDefault();
      usePlayer.getState().toggle();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [available]);

  if (!available) {
    return (
      <div role="status" className="flex h-11 items-center gap-2 border-t border-line bg-surface px-4 text-[13px] text-muted">
        <Icon name="volume_off" size={18} />
        {t("audio.removed")}
      </div>
    );
  }

  const usable = Boolean(src) && !error;
  // Seeks from the waveform and ±15 s keep playing or staying paused as they were.
  const jump = (ms: number) => usePlayer.getState().seek(ms, usePlayer.getState().playing);

  return (
    <div role="group" data-testid="audio-bar" aria-label={t("audio.label")} className="flex min-h-16 items-center gap-3.5 border-t border-line bg-surface px-6 py-1.5">
      <audio
        ref={audio}
        src={src ?? undefined}
        preload="metadata"
        onTimeUpdate={report}
        onSeeked={report}
        onPlay={() => usePlayer.getState().reportPlaying(true)}
        onPause={() => usePlayer.getState().reportPlaying(false)}
        onEnded={() => usePlayer.getState().reportPlaying(false)}
        onError={onError}
      />
      <button type="button" data-testid="audio-play" disabled={!usable} onClick={() => usePlayer.getState().toggle()} aria-label={playing ? t("audio.pause") : t("audio.play")} className={playBtn}>
        <Icon name={playing ? "pause_fill" : "play_arrow_fill"} size={24} />
      </button>
      <Time durationMs={durationMs} />
      {error ? (
        <p role="alert" className="m-0 flex-1 text-[13px] text-warn">
          {t("audio.error")}
        </p>
      ) : loading || wave.isPending ? (
        <div data-testid="waveform-loading" role="status" aria-label={t("audio.computing")} className="h-[38px] flex-1 animate-pulse rounded-seg bg-sunk motion-reduce:animate-none" />
      ) : (
        <WaveformSlider data={wave.data} segments={segments} speakers={detail.speakers} durationMs={durationMs} seekTo={jump} />
      )}
      <Menu
        label={t("audio.speed")}
        trigger={
          <button type="button" disabled={!usable} aria-label={t("audio.speedNow", { rate })} className="text-mono h-[30px] min-w-11 shrink-0 rounded-ctl border border-ctl bg-surface px-2 text-[12px] hover:bg-sunk disabled:opacity-40">
            {`${rate}×`}
          </button>
        }
        items={RATES.map((r) => ({ label: `${r}×`, icon: r === rate ? "check" : undefined, onSelect: () => usePlayer.getState().setRate(r) }))}
      />
      <button
        type="button"
        aria-pressed={skipSilence}
        disabled={!usable}
        onClick={() => usePlayer.getState().setSkipSilence(!skipSilence)}
        className={cn("inline-flex h-[30px] shrink-0 items-center gap-1 rounded-ctl border border-ctl px-2.5 text-[12px] font-medium disabled:opacity-40", skipSilence ? "bg-accent-soft text-accent" : "text-muted hover:bg-sunk")}
      >
        <Icon name="fast_forward" size={16} />
        <span className="max-[1100px]:sr-only">{t("detail.skipSilence")}</span>
      </button>
    </div>
  );
}

/** Current / total; its own component so only it re-renders with the time. */
function Time({ durationMs }: { durationMs: number }) {
  const ms = usePlayer((s) => s.currentMs);
  return (
    <span className="text-mono min-w-[92px] shrink-0 text-[12px] whitespace-nowrap text-muted tabular-nums" aria-hidden="true">
      {`${formatClock(ms, { pad: true })} / ${formatClock(durationMs, { pad: true })}`}
    </span>
  );
}
