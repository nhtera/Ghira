// SPDX-License-Identifier: Apache-2.0
// D2: the menu bar popover. Start (Call/Room), the current recording with
// Stop/Pause/Open, the last three meetings, the privacy line, Open {{app}}.
// It is its own window: recording or navigating happens in the main window
// (`showMain`), then the popover hides.
import { NextEvent } from "../calendar/next-event";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { formatClock, formatTime, type Locale } from "@ghi/i18n";
import {
  Button,
  Icon,
  PrivacyIndicator,
  cn,
  type IconName,
  type RecordMode,
} from "@ghi/ui";
import type { MeetingRow } from "../../bindings";
import { ipc } from "../../ipc";
import { elapsedMs, isActive, useLive } from "../../state/live";
import { useNow } from "../live/clock";
import { usePanelWindow } from "./panel-window";

type Result = { status: "ok" } | { status: "error"; error: string };

const TILE =
  "flex h-14 flex-col items-center justify-center gap-0.5 rounded-[10px] text-[13px] font-semibold";

/** The row's source glyph and its state glyph, as in the library. */
function rowIcons(m: MeetingRow): { source: IconName; state: IconName; warn: boolean; label: "failedRetry" | "processing" | "cloudEnhanced" | "ready" } {
  const source = m.source === "import" ? "description" : m.mode === "room" ? "groups" : "videocam";
  if (m.status === "failed") return { source, state: "warning", warn: true, label: "failedRetry" };
  if (m.job || m.status === "processing") return { source, state: "sync", warn: false, label: "processing" };
  if (m.cloudUsed) return { source, state: "cloud", warn: true, label: "cloudEnhanced" };
  return { source, state: "check_circle", warn: false, label: "ready" };
}

export function Popover() {
  const { t, i18n } = useTranslation();
  const locale = (i18n.language === "vi" ? "vi" : "en") as Locale;
  usePanelWindow();
  const state = useLive((s) => s.state);
  const title = useLive((s) => s.session?.title);
  const clock = useLive(
    useShallow((s) => ({
      startedAtMs: s.startedAtMs,
      pausedAtMs: s.pausedAtMs,
      pausedTotalMs: s.pausedTotalMs,
    })),
  );
  const now = useNow(state === "recording");
  const [error, setError] = useState<string | null>(null);
  const speakers = useLive((s) => Object.keys(s.speakers).length);
  const active = isActive(state);

  const recent = useQuery({
    queryKey: ["popover-recent"],
    staleTime: 0,
    queryFn: async () => {
      const r = await ipc.commands.listMeetings(3, 0);
      if (r.status === "error") throw new Error(r.error);
      return r.data;
    },
  });

  const hide = () => void ipc.commands.hidePopover();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && hide();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const guard = async (p: Promise<Result>) => {
    const r = await p;
    if (r.status === "error") setError(r.error);
    else setError(null);
    return r.status === "ok";
  };
  const goMain = async (route: string | null) => {
    if (await guard(ipc.commands.showMain(route))) hide();
  };
  const start = async (m: RecordMode) => {
    if (await guard(ipc.commands.startRecording(m, null, "")))
      await goMain("/live");
  };

  return (
    <main className="box-border flex h-screen flex-col gap-3 overflow-hidden bg-surface p-3.5 text-ink">
      {active ? (
        <section
          aria-label={t("tray.recording")}
          className="flex flex-col gap-2.5 rounded-row bg-rec-soft p-3"
        >
          <div className="flex items-center gap-2 text-[13px] font-semibold text-rec-ink">
            <span aria-hidden="true" className="size-2 rounded-full bg-rec" />
            <span className="min-w-0 flex-1 truncate">
              {title || t("live.titlePlaceholder")}
            </span>
            <span className="text-mono font-medium">
              {formatClock(elapsedMs(clock, now))}
            </span>
          </div>
          {speakers > 0 && (
            <p className="text-small m-0 text-rec-ink">
              {t("live.speakerCount", { count: speakers })}
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="danger"
              icon="stop"
              onClick={() => void guard(ipc.commands.stopRecording())}
            >
              {t("live.stop")}
            </Button>
            <Button
              size="sm"
              icon="star"
              disabled={state !== "recording"}
              onClick={() => void guard(ipc.commands.markMoment())}
            >
              {t("live.mark")}
            </Button>
            {state === "paused" ? (
              <Button
                size="sm"
                icon="play_arrow"
                onClick={() => void guard(ipc.commands.resumeRecording())}
              >
                {t("live.resume")}
              </Button>
            ) : (
              <Button
                size="sm"
                icon="pause"
                disabled={state !== "recording"}
                onClick={() => void guard(ipc.commands.pauseRecording())}
              >
                {t("live.pause")}
              </Button>
            )}
            <Button
              size="sm"
              variant="ghost"
              icon="open_in_new"
              onClick={() => void goMain("/live")}
            >
              {t("common.open")}
            </Button>
          </div>
        </section>
      ) : (
        <div className="grid grid-cols-2 gap-1.5">
          <button
            type="button"
            onClick={() => void start("call")}
            className={cn(TILE, "bg-accent text-on-accent hover:brightness-110")}
          >
            <Icon name="videocam" size={20} />
            {t("tray.recordCall")}
          </button>
          <button
            type="button"
            onClick={() => void start("room")}
            className={cn(TILE, "border border-ctl bg-surface hover:bg-surface2")}
          >
            <Icon name="groups" size={20} />
            {t("tray.recordRoom")}
          </button>
        </div>
      )}

      {error && (
        <p role="alert" className="text-small m-0 text-rec-ink">
          {t("system.commandFailed", { message: error })}
        </p>
      )}

      <NextEvent />

      <section
        aria-label={t("tray.recent")}
        className="flex min-h-0 flex-1 flex-col gap-1 overflow-hidden"
      >
        <h2 className="m-0 px-1.5 text-[11px] font-semibold tracking-[.06em] text-faint uppercase">
          {t("tray.recent")}
        </h2>
        <ul className="m-0 flex list-none flex-col gap-px p-0">
          {(recent.data ?? []).map((m) => (
            <li key={m.gid}>
              <button
                type="button"
                onClick={() => void goMain(`/meetings/${m.gid}/notes`)}
                className="flex min-h-8 w-full items-center gap-2 rounded-seg px-1.5 py-1 text-left text-[13px] hover:bg-surface2"
              >
                <Icon name={rowIcons(m).source} size={16} className="flex-none text-faint" />
                <span className="min-w-0 flex-1 truncate">
                  {m.title || t("live.titlePlaceholder")}
                </span>
                <span className="sr-only">
                  {m.startedAt != null && formatTime(m.startedAt, locale)}
                </span>
                <Icon
                  name={rowIcons(m).state}
                  size={15}
                  label={t(`library.status.${rowIcons(m).label}`)}
                  className={cn("flex-none", rowIcons(m).warn ? "text-warn" : "text-muted")}
                />
              </button>
            </li>
          ))}
        </ul>
      </section>

      <footer className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1.5 border-t border-line pt-2.5">
        <PrivacyIndicator
          state={state === "paused" ? "paused" : active ? "recording" : "local"}
        />
        <div className="ml-auto flex items-center gap-1">
          <Button size="sm" onClick={() => void goMain(null)}>
            {t("tray.open")}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => void ipc.commands.requestQuitApp()}
          >
            {t("tray.quit")}
          </Button>
        </div>
      </footer>
    </main>
  );
}
