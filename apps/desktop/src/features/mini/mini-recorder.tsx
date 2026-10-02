// SPDX-License-Identifier: Apache-2.0
// D4: the mini recorder, a small always-on-top window. Full (360x120): state
// dot, timer, mic level, the latest line, Mark/Pause/Stop and Expand/Collapse.
// Pill (180x44): dot, timer, Expand. The native window is resized by
// `setMiniCompact`; this page only renders what fits.
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { formatClock } from "@ghi/i18n";
import { Avatar, Button, LevelMeter, cn } from "@ghi/ui";
import { useLock } from "../../state/lock";
import { ipc } from "../../ipc";
import { elapsedMs, isActive, useLive } from "../../state/live";
import { speakerNumber, useSpeakerLabel } from "../../state/speaker-label";
import { useNow } from "../live/clock";
import { usePanelWindow } from "../popover/panel-window";

const GRACE_MS = 1500; // a mini opened before the snapshot arrived is not "ended"

export function MiniRecorder() {
  const { t } = useTranslation();
  usePanelWindow();
  const state = useLive((s) => s.state);
  const clock = useLive(
    useShallow((s) => ({
      startedAtMs: s.startedAtMs,
      pausedAtMs: s.pausedAtMs,
      pausedTotalMs: s.pausedTotalMs,
    })),
  );
  // No level yet (starting, paused, asleep) is silence; "no microphone" is only for a lost device.
  const mic = useLive((s) =>
    s.capture.lostTracks.includes(0) ? null : (s.levels.mic ?? -100),
  );
  const text = useLive(
    (s) => s.partial[0] || s.partial[1] || s.lines.at(-1)?.text || "",
  );
  const speaker = useLive((s) => {
    const id = s.lines.at(-1)?.speaker;
    return id == null ? undefined : s.speakers[id];
  });
  const speakerLabel = useSpeakerLabel();
  // Locked: the controls stay, the words don't.
  const locked = useLock((st) => st.locked === true);
  const now = useNow(state === "recording");
  // Opens as a pill (`#/mini?compact=1`): a full window would show text on a shared screen.
  const [compact, setCompact] = useState(
    () =>
      new URLSearchParams(window.location.hash.split("?")[1] ?? "").get(
        "compact",
      ) === "1",
  );
  const [error, setError] = useState<string | null>(null);
  const seenActive = useRef(false);

  // The session ended (processing, ready, idle): nothing left to control.
  useEffect(() => {
    if (isActive(state)) {
      seenActive.current = true;
      return;
    }
    if (seenActive.current) return void ipc.commands.closeMini();
    const id = window.setTimeout(() => void ipc.commands.closeMini(), GRACE_MS);
    return () => window.clearTimeout(id);
  }, [state]);

  const run = async (
    p: Promise<{ status: "ok" } | { status: "error"; error: string }>,
  ) => {
    const r = await p;
    setError(r.status === "error" ? r.error : null);
    return r.status === "ok";
  };
  const resize = async (next: boolean) => {
    if (await run(ipc.commands.setMiniCompact(next))) setCompact(next);
  };
  const expand = async () => {
    if (await run(ipc.commands.showMain("/live")))
      void ipc.commands.closeMini();
  };

  const paused = state === "paused";
  const timer = formatClock(elapsedMs(clock, now));
  const dot = (
    <span
      aria-hidden="true"
      className={cn(
        "size-2.5 flex-none rounded-full",
        paused ? "bg-muted" : "bg-rec animate-pulse motion-reduce:animate-none",
      )}
    />
  );
  const status = (
    <span className="sr-only">
      {t(paused ? "privacy.paused" : "tray.recording")}
    </span>
  );
  const time = (
    <span className="text-mono text-[13px] font-semibold tabular-nums">
      {timer}
    </span>
  );

  if (compact) {
    return (
      <main
        data-tauri-drag-region
        className="box-border flex h-screen items-center gap-2 overflow-hidden bg-surface px-3.5 text-ink"
      >
        {dot}
        {status}
        {time}
        <span data-tauri-drag-region className="flex-1" />
        <Button
          size="sm"
          variant="ghost"
          icon="open_in_full"
          aria-label={t("mini.expand")}
          title={t("mini.expand")}
          onClick={() => void resize(false)}
        />
      </main>
    );
  }
  return (
    <main
      data-tauri-drag-region
      className="box-border flex h-screen flex-col justify-between overflow-hidden bg-surface px-3 py-2.5 text-ink"
    >
      <div data-tauri-drag-region className="flex items-center gap-2">
        {dot}
        {status}
        {time}
        <span data-tauri-drag-region className="flex-1" />
        <Button
          size="sm"
          variant="ghost"
          icon="star"
          aria-label={t("live.mark")}
          title={t("live.mark")}
          disabled={!isActive(state)}
          onClick={() => void run(ipc.commands.markMoment())}
        />
        {paused ? (
          <Button
            size="sm"
            variant="ghost"
            icon="play_arrow"
            aria-label={t("live.resume")}
            title={t("live.resume")}
            onClick={() => void run(ipc.commands.resumeRecording())}
          />
        ) : (
          <Button
            size="sm"
            variant="ghost"
            icon="pause"
            aria-label={t("live.pause")}
            title={t("live.pause")}
            disabled={state !== "recording"}
            onClick={() => void run(ipc.commands.pauseRecording())}
          />
        )}
        <Button
          size="sm"
          variant="danger"
          icon="stop"
          aria-label={t("live.stop")}
          title={t("live.stop")}
          onClick={() => void run(ipc.commands.stopRecording())}
        />
        <Button
          size="sm"
          variant="ghost"
          icon="picture_in_picture_alt"
          aria-label={t("mini.collapse")}
          title={t("mini.collapse")}
          onClick={() => void resize(true)}
        />
        <Button
          size="sm"
          variant="ghost"
          icon="open_in_new"
          aria-label={t("common.open")}
          title={t("common.open")}
          onClick={() => void expand()}
        />
      </div>
      <p
        data-tauri-drag-region
        className="m-0 flex items-center gap-1.5 text-[13px] text-muted"
      >
        {!error && !locked && speaker && (
          <>
            <Avatar
              name={speakerLabel(speaker)}
              initial={speakerNumber(speaker) ?? undefined}
              kind={speaker.isMe ? "me" : "person"}
              colorSlot={speaker.colorSlot}
              size="sm"
            />
            <span className="flex-none font-semibold text-ink">
              {speakerLabel(speaker)}
            </span>
          </>
        )}
        <span className="min-w-0 truncate">
          {error ? t("system.commandFailed", { message: error }) : locked ? t("system.locked.title") : text}
        </span>
      </p>
      <LevelMeter db={mic} source="mic" />
    </main>
  );
}
