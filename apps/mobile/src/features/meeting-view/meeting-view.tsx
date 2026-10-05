// SPDX-License-Identifier: Apache-2.0
// M4: one meeting, read first, laid out like the design: back and share on the
// bar, the title below it (it collapses into the bar on scroll), a segmented
// Notes / Actions / Transcript control, and the audio bar with its
// speaker-coloured waveform. Privacy and the sync chip sit under the title.
import { formatDate } from "@ghi/i18n";
import { Button, cn, Icon, NavBar, PrivacyIndicator, SyncChip, useLargeTitleCollapse } from "@ghi/ui";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Citation } from "../../bindings";
import { useLocale, useMeetingWhen } from "../meeting-list/format";
import { ActionsPanel } from "./actions-panel";
import { AudioBar, AudioOnDevice } from "./audio-bar";
import { NotesPanel } from "./notes-panel";
import { speakerOf, transcriptSpeaker } from "./notes-model";
import { QuoteSheet } from "./quote-sheet";
import { ShareSheet } from "./share-sheet";
import { TranscriptPanel } from "./transcript-panel";
import { useAudio } from "./use-audio";
import { useWaveform } from "./use-waveform";
import { LOCKED_EVENT } from "../app-lock/events";
import { useWindowEvent } from "./use-window-event";
import { useMeeting } from "./use-meeting";
import { Switch } from "../settings/controls";
import { SensitiveBadge, SensitiveRow, SensitiveSheet } from "../sensitive";
import { ConflictBanner, ProcessHereSheet, RefiningBanner, useMeetingSync } from "../sync";

export type MeetingTab = "notes" | "actions" | "transcript";
const TABS: MeetingTab[] = ["notes", "actions", "transcript"];

export type MeetingViewProps = { id: string; tab?: MeetingTab; at?: number };

export function MeetingView({
  id,
  tab: initial = "notes",
  at,
}: MeetingViewProps) {
  const { t } = useTranslation();
  const locale = useLocale();
  const when = useMeetingWhen();
  const navigate = useNavigate();
  const m = useMeeting(id);
  const sync = useMeetingSync(id, m.detail, m.chip, m.reload);
  const [taking, setTaking] = useState(false);
  const audio = useAudio(id, m.detail?.durationMs ?? null);
  const wave = useWaveform(id, Boolean(m.detail?.audioAvailable));
  const ready = m.status === "ready" && Boolean(m.detail);
  const { collapsed, scrollRef: scroller, titleRef } = useLargeTitleCollapse(ready);
  const [tab, setTab] = useState<MeetingTab>(initial);
  const [quote, setQuote] = useState<{
    citation: Citation;
    key: string;
  } | null>(null);
  const [visited, setVisited] = useState<ReadonlySet<string>>(new Set());
  const [sharing, setSharing] = useState(false);
  const [asking, setAsking] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  // Locking hides the meeting: its sheets close with it.
  useWindowEvent(LOCKED_EVENT, () => {
    setQuote(null);
    setSharing(false);
    setAsking(false);
    setTaking(false);
  });

  const back = () => void navigate({ to: "/meetings" });
  const label: Record<MeetingTab, string> = {
    notes: t("mobile.detail.notes"),
    actions: t("mobile.detail.actions"),
    transcript: t("mobile.detail.transcript"),
  };

  if (m.status !== "ready" || !m.detail) {
    return (
      <section data-screen="meeting" className="flex h-full flex-col bg-surface">
        <NavBar
          className="bg-surface"
          large={false}
          title={t("mobile.meetings.title")}
          onBack={back}
        />
        <div
          role="status"
          className="flex flex-col items-center gap-3 px-6 py-10 text-center"
        >
          <p className="text-ios-subhead m-0 text-muted">
            {m.status === "missing"
              ? t("mobile.detail.notFound")
              : m.status === "locked"
                ? t("mobile.detail.locked")
                : m.status === "failed"
                  ? t("mobile.detail.loadFailed")
                  : t("mobile.meetings.loading")}
          </p>
          {m.status === "failed" && (
            <Button
              variant="primary"
              className="min-h-ios-target px-5"
              onClick={m.reload}
            >
              {t("mobile.meetings.retry")}
            </Button>
          )}
        </div>
      </section>
    );
  }

  const { detail } = m;
  const title = detail.title || t("mobile.meetings.untitled");
  const quoteSpeaker =
    transcriptSpeaker(
      speakerOf(detail.speakers, quote?.citation.speakerGid ?? null),
      (number) => t("speakers.numbered", { number }),
      t("speakers.me"),
    ) ?? undefined;
  const cite = (citation: Citation, key: string) => setQuote({ citation, key });

  return (
    <section data-screen="meeting" className="flex h-full flex-col bg-surface">
      <NavBar
        className="bg-surface"
        // Empty until the heading below has scrolled away: the hidden copy would take the back label's room.
        title={collapsed ? title : ""}
        collapsed={collapsed}
        onBack={back}
        backLabel={t("mobile.meetings.title")}
        trailing={
          <button
            type="button"
            onClick={() => setSharing(true)}
            aria-label={t("mobile.detail.share")}
            className="grid min-h-ios-target min-w-ios-target place-items-center text-accent"
          >
            <Icon name="ios_share" size={22} />
          </button>
        }
      />
      <div ref={scroller} className="relative min-h-0 flex-1 overflow-y-auto">
        <div className="flex flex-col gap-1.5 px-4 pt-1 pb-2">
          <h1 ref={titleRef} className="text-ios-title1 m-0 break-words">
            {title}
          </h1>
          <p className="text-ios-subhead m-0 text-muted">
            {[
              detail.startedAt === null
                ? null
                : formatDate(detail.startedAt, locale),
              when(detail.startedAt, detail.durationMs),
            ]
              .filter(Boolean)
              .join(" · ")}
          </p>
          <div className="flex flex-wrap items-center gap-2">
            {m.chip && <SyncChip chip={m.chip} device={sync.device ?? undefined} />}
            <PrivacyIndicator
              state={detail.cloudUsed ? "cloudMeeting" : "local"}
            />
            {detail.sensitive && <SensitiveBadge />}
          </div>
        </div>
        {sync.conflict && (
          <ConflictBanner
            device={sync.conflict.device}
            text={sync.conflict.text}
            busy={sync.busy}
            error={sync.error}
            onUse={() => void sync.resolve(true)}
            onDismiss={() => void sync.resolve(false)}
          />
        )}
        <div className="sticky top-0 z-10 bg-surface px-4 pt-1 pb-2">
          <div
            role="tablist"
            aria-label={t("mobile.detail.tabs")}
            className="flex gap-0.5 rounded-(--ios-radius-group) bg-sunk p-0.5"
          >
            {TABS.map((k) => (
              <button
                key={k}
                type="button"
                role="tab"
                id={`tab-${k}`}
                aria-selected={tab === k}
                aria-controls={tab === k ? `panel-${k}` : undefined}
                onClick={() => setTab(k)}
                className={cn(
                  "text-ios-subhead min-h-ios-target flex-1 rounded-[10px] px-2 font-semibold",
                  tab === k ? "bg-surface text-ink shadow-sm" : "text-muted",
                )}
              >
                {label[k]}
              </button>
            ))}
          </div>
        </div>
        {tab === "transcript" && sync.leaseOpen && sync.device && (
          <RefiningBanner
            device={sync.device}
            onProcessHere={() => {
              sync.clearError();
              setTaking(true);
            }}
          />
        )}
        <div role="tabpanel" id={`panel-${tab}`} aria-labelledby={`tab-${tab}`}>
          {tab === "notes" && (
            <>
              <NotesPanel
                meeting={id}
                cloudLocked={detail.cloudLocked || detail.sensitive}
                onSent={m.reload}
                notes={m.notes}
                visited={visited}
                onCite={cite}
              />
              <div className="mx-4 mt-2 mb-4 flex flex-col gap-3 rounded-(--ios-radius-group) bg-surface2 p-3">
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0">
                    <p id="cloud-never" className="text-ios-subhead m-0">
                      {t("mobile.detail.cloudNever")}
                    </p>
                    <p className="text-ios-footnote m-0 text-muted">
                      {t("mobile.detail.cloudNeverHint")}
                    </p>
                  </div>
                  <Switch
                    checked={detail.cloudLocked || detail.sensitive}
                    disabled={detail.sensitive}
                    onChange={(on) => void m.setCloudLocked(on)}
                    labelledBy="cloud-never"
                  />
                </div>
                <div className="border-t border-line pt-3">
                  <SensitiveRow
                    checked={detail.sensitive}
                    // On asks first (the audio is deleted now); off needs no question.
                    onChange={(on) => {
                      setFailed(null);
                      if (on) setAsking(true);
                      else void m.setSensitive(false).then(setFailed);
                    }}
                  />
                  {failed && (
                    <p role="alert" className="text-ios-footnote m-0 mt-1 text-warn">
                      {failed === "noTranscript" ? t("mobile.sensitive.noTranscript") : failed === "transcriptPending" ? t("mobile.sensitive.pending") : t("mobile.sensitive.failed")}
                    </p>
                  )}
                </div>
              </div>
            </>
          )}
          {tab === "actions" && (
            <ActionsPanel
              notes={m.notes}
              speakers={detail.speakers}
              visited={visited}
              onToggle={(item, done) => void m.setActionDone(item, done)}
              onCite={cite}
            />
          )}
          {tab === "transcript" && (
            <TranscriptPanel
              // The banner above moves the list: start it afresh when the lease opens or closes.
              key={String(sync.leaseOpen)}
              readOnly={sync.leaseOpen}
              segments={m.transcript.segments}
              speakers={detail.speakers}
              scroller={scroller}
              timeMs={audio.timeMs}
              playing={audio.playing}
              focusAt={at}
              canPlay={detail.audioAvailable}
              onPlay={(ms) => void audio.playFrom(ms)}
              onSave={m.saveSegment}
              empty={
                detail.job?.waitingForModels
                  ? t("mobile.detail.transcriptWaiting")
                  : t("mobile.detail.transcriptEmpty")
              }
            />
          )}
        </div>
      </div>
      {detail.audioAvailable ? (
        <AudioBar
          audio={audio}
          wave={wave}
          segments={m.transcript.segments}
          speakers={detail.speakers}
        />
      ) : (
        sync.audioOnDevice && sync.device && <AudioOnDevice device={sync.device} />
      )}
      <ProcessHereSheet
        open={taking && sync.device !== null}
        device={sync.device ?? ""}
        busy={sync.busy}
        error={sync.error}
        onCancel={() => setTaking(false)}
        onConfirm={() => void sync.processHere().then((ok) => ok && setTaking(false))}
      />
      <QuoteSheet
        citation={quote?.citation ?? null}
        speaker={quoteSpeaker}
        onClose={() => {
          if (quote) setVisited((v) => new Set(v).add(quote.key));
          setQuote(null);
        }}
        canPlay={detail.audioAvailable}
        onPlay={(ms) => void audio.playFrom(ms)}
      />
      <SensitiveSheet
        open={asking}
        recording={false}
        onCancel={() => setAsking(false)}
        onConfirm={() => {
          setAsking(false);
          void m.setSensitive(true).then(setFailed);
        }}
      />
      <ShareSheet
        open={sharing}
        onClose={() => setSharing(false)}
        meeting={id}
      />
    </section>
  );
}
