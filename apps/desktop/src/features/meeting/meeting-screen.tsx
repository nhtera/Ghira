// SPDX-License-Identifier: Apache-2.0
// The meeting detail screen (D6): header, toolbar, Notes | Transcript tabs and
// the audio bar docked under both. Also the states around it: loading, not
// found, still recording, notes being written, notes failed, no audio.
import {
  Button,
  Icon,
  cn,
  usePlatform,
  useToast,
} from "@ghi/ui";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingDetail } from "../../bindings";
import { ipc } from "../../ipc";
import {
  invalidateMeeting,
  useMeetingDetail,
  useMeetingEvents,
} from "../../state/meeting-queries";
import { usePlayer } from "../../state/player";
import { AskPanel } from "../ask-meeting/ask-panel";
import { CloudSheet } from "../cloud-sheet/cloud-sheet";
import { AudioBar } from "../audio-bar/audio-bar";
import { ExportSheet } from "../export/export-sheet";
import { NotesTab } from "../notes/notes-tab";
import { TranscriptTab } from "../transcript/transcript-tab";
import { useProcessing } from "../processing/processing-store";
import { MeetingProcessing } from "../processing/meeting-processing";
import { ConflictBanner } from "../sync/conflict-banner";
import { MeetingHeader } from "./meeting-header";
import { MeetingToolbar } from "./meeting-toolbar";
import { inProgress } from "../library/meeting-status";

export type DetailTab = "notes" | "transcript";

function Notice({
  icon,
  title,
  body,
  children,
}: {
  icon: "error" | "search_off" | "radio_button_checked";
  title: string;
  body?: string;
  children?: React.ReactNode;
}) {
  return (
    <div
      role="status"
      className="m-7 flex max-w-xl flex-col items-start gap-2 rounded-panel border border-line2 bg-surface p-5"
    >
      <Icon name={icon} size={24} className="text-muted" />
      <b className="text-heading">{title}</b>
      {body && <p className="text-body m-0 text-muted">{body}</p>}
      {children}
    </div>
  );
}

function FailedBanner({ detail }: { detail: MeetingDetail }) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const client = useQueryClient();
  const { show } = useToast();
  const retry = async () => {
    const r = await ipc.commands.retryMeeting(detail.gid);
    if (r.status === "error")
      return show({
        tone: "warning",
        title: t("system.commandFailed", { message: r.error }),
      });
    if (r.data === 0)
      show({ tone: "info", title: t("library.nothingToRetry") });
    void invalidateMeeting(client, detail.gid);
  };
  return (
    <div
      role="alert"
      className="mx-7 mt-2 flex flex-col items-start gap-2 rounded-panel border-[1.5px] border-line2 bg-surface p-4"
    >
      <b className="text-body font-semibold">{t("detail.failed.title")}</b>
      <p className="text-small m-0 text-muted">{t("detail.failed.body")}</p>
      <Button
        variant="primary"
        size="sm"
        icon="refresh"
        onClick={() => void retry()}
      >
        {t("detail.failed.retryLocal", { context: platform })}
      </Button>
    </div>
  );
}

/**
 * Mirrors playback time onto the screen root (e2e reads `data-player-ms`)
 * without a React subscription: the screen must not re-render every frame.
 */
function PlayerProbe() {
  const ref = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    const root = ref.current?.parentElement;
    if (!root) return;
    const write = (s: { currentMs: number; playing: boolean }) => {
      root.dataset.playerMs = String(Math.round(s.currentMs));
      root.dataset.playerPlaying = s.playing ? "true" : "false";
    };
    write(usePlayer.getState());
    return usePlayer.subscribe(write);
  }, []);
  return <span ref={ref} hidden />;
}

export function MeetingScreen({
  id,
  tab,
  startAtMs,
}: {
  id: string;
  tab: DetailTab;
  /** Opened from a search hit. */
  startAtMs?: number;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const q = useMeetingDetail(id);
  const detail = q.data;
  const [exporting, setExporting] = useState(false);
  const [cloudOpen, setCloudOpen] = useState(false);
  const [asking, setAsking] = useState(false);
  const [onlyMine, setOnlyMine] = useState(false);
  // Notes finished this session: "Name your speakers" stays after the stepper is gone.
  const finished = useProcessing((s) => s.finished.includes(id));
  useMeetingEvents(id);

  const hasAudio =
    !!detail && detail.audioAvailable && detail.status !== "recording";
  useEffect(() => {
    if (!hasAudio) return;
    void usePlayer.getState().load(id);
    return () => usePlayer.getState().unload();
  }, [id, hasAudio]);

  const goTab = (next: DetailTab) =>
    void navigate({ to: "/meetings/$id/$tab", params: { id, tab: next } });

  if (q.isPending) {
    return (
      <div aria-busy="true" className="flex flex-col gap-3 p-7">
        <div className="h-8 w-1/2 animate-pulse rounded-seg bg-sunk motion-reduce:animate-none" />
        <div className="h-4 w-1/3 animate-pulse rounded-seg bg-sunk motion-reduce:animate-none" />
        <div className="h-40 animate-pulse rounded-panel bg-sunk motion-reduce:animate-none" />
      </div>
    );
  }
  if (q.isError && !detail) {
    return (
      <Notice
        icon="error"
        title={t("system.commandFailed", { message: q.error.message })}
      >
        <Button icon="refresh" onClick={() => void q.refetch()}>
          {t("common.tryAgain")}
        </Button>
      </Notice>
    );
  }
  if (!detail) {
    return (
      <Notice
        icon="search_off"
        title={t("meeting.notFound.title")}
        body={t("meeting.notFound.body")}
      >
        <Button
          icon="arrow_back"
          onClick={() => void navigate({ to: "/meetings" })}
        >
          {t("detail.back")}
        </Button>
      </Notice>
    );
  }
  if (detail.status === "recording") {
    return (
      <Notice
        icon="radio_button_checked"
        title={t("meeting.stillRecording.title")}
        body={t("meeting.stillRecording.body")}
      >
        <Button
          variant="primary"
          onClick={() => void navigate({ to: "/live" })}
        >
          {t("meeting.stillRecording.open")}
        </Button>
      </Notice>
    );
  }

  const busy = inProgress(detail.status) || detail.job != null;
  const tabs: { id: DetailTab; label: string }[] = [
    { id: "notes", label: t("notes.tab") },
    { id: "transcript", label: t("notes.transcriptTab") },
  ];

  return (
    <div data-testid="meeting-detail" className="flex h-full min-h-0 flex-col">
      <PlayerProbe />
      <div className="flex min-h-0 flex-1">
        <div className="min-h-0 min-w-0 flex-1 overflow-auto">
          <MeetingHeader detail={detail} />
          {detail.status === "failed" && <FailedBanner detail={detail} />}
          <ConflictBanner meeting={id} />
          <MeetingToolbar
            detail={detail}
            onExport={() => setExporting(true)}
            onImproveWithCloud={() => setCloudOpen(true)}
            onAsk={() => setAsking((a) => !a)}
            onlyMine={tab === "notes" ? onlyMine : undefined}
            onOnlyMine={tab === "notes" ? setOnlyMine : undefined}
            tabs={
              <div
                role="tablist"
                aria-label={t("meeting.tabs")}
                className="flex gap-2.5"
              >
                {tabs.map((x) => (
                  <button
                    key={x.id}
                    type="button"
                    role="tab"
                    id={`tab-${x.id}`}
                    aria-selected={tab === x.id}
                    aria-controls="detail-panel"
                    tabIndex={tab === x.id ? 0 : -1}
                    onClick={() => goTab(x.id)}
                    onKeyDown={(e) => {
                      if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
                      const next =
                        tabs[
                          (tabs.findIndex((y) => y.id === x.id) + 1) % tabs.length
                        ]!;
                      e.preventDefault();
                      goTab(next.id);
                      document.getElementById(`tab-${next.id}`)?.focus();
                    }}
                    className={cn(
                      "-mb-px h-[38px] border-b-2 px-3 text-[13.5px] font-semibold",
                      tab === x.id
                        ? "border-accent text-accent"
                        : "border-transparent text-muted hover:text-ink",
                    )}
                  >
                    {x.label}
                  </button>
                ))}
              </div>
            }
          />
          <div
            role="tabpanel"
            id="detail-panel"
            aria-labelledby={`tab-${tab}`}
            className="px-7 pt-5"
          >
            <div className="max-w-[760px]">
              {tab === "notes" ? (
                <>
                  {((busy && detail.status !== "failed") || finished) && (
                    <MeetingProcessing meeting={id} waitingForModels={detail.job?.waitingForModels} job={detail.job} />
                  )}
                  <NotesTab meeting={id} detail={detail} onlyMine={onlyMine} />
                </>
              ) : (
                <TranscriptTab
                  meeting={id}
                  detail={detail}
                  startAtMs={startAtMs}
                />
              )}
            </div>
          </div>
        </div>
        {asking && (
          <AskPanel
            meeting={id}
            detail={detail}
            onClose={() => setAsking(false)}
          />
        )}
      </div>
      {hasAudio && <AudioBar meeting={id} detail={detail} />}
      <ExportSheet
        open={exporting}
        onOpenChange={setExporting}
        meetings={[id]}
      />
      <CloudSheet
        open={cloudOpen}
        onOpenChange={setCloudOpen}
        meeting={id}
        locked={detail.cloudLocked || detail.sensitive}
        task={{ kind: "notes", template: detail.template }}
      />
    </div>
  );
}
