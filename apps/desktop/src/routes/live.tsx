// SPDX-License-Identifier: Apache-2.0
// Live meeting (D4): header (title, mode, record control, layout, mark,
// consent), speaker strip and lanes, the virtualized transcript with the
// notepad beside it (Transcript layout) or in front (Focus layout), health
// and system states as inline banners. Pieces live in `features/live`.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Menu, Segmented, usePlatform } from "@ghi/ui";
import type { RecordMode } from "../bindings";
import { useAppActions } from "../shell/actions";
import { Page } from "../shell/page";
import { useCompact } from "../shell/use-compact";
import { DiscardPanel, FocusCaption, HeaderRecord, Health, LiveBanners, Levels, LiveToolbar, Notepad, SpeakerStrip, TranscriptView } from "../features/live";
import { isActive, useLive } from "../state/live";
import { useUi } from "../state/ui";

type Layout = "transcript" | "focus";
const DISCARD_SECONDS = [60, 300, 600];

export function LiveScreen() {
  const { t } = useTranslation();
  const platform = usePlatform();
  const compact = useCompact();
  const state = useLive((s) => s.state);
  const meeting = useLive((s) => s.meeting);
  const session = useLive((s) => s.session);
  const marks = useLive((s) => s.marks.length);
  const { run } = useAppActions();
  const pendingMode = useUi((s) => s.recordMode);
  const [layout, setLayout] = useState<Layout>("transcript");
  const [discard, setDiscard] = useState<number | null>(null);
  const active = isActive(state) && meeting != null;
  const recording = state === "recording" || state === "paused";

  return (
    <Page
      title={t("nav.live")}
      subtitle={compact ? undefined : t("live.localLine", { context: platform })}
      className="flex min-h-0 flex-col gap-3 overflow-hidden"
      actions={
        <div className="flex items-center gap-2">
          {recording && (
            <>
              <Segmented
                label={t("live.layout")}
                value={layout}
                onChange={setLayout}
                options={[
                  { value: "transcript", label: t("live.layoutTranscript"), icon: "subtitles" },
                  { value: "focus", label: t("live.layoutFocus"), icon: "edit" },
                ]}
              />
              <Button icon="star" onClick={() => void run("mark")} aria-label={compact ? t("live.mark") : undefined} title={compact ? t("live.mark") : undefined}>
                {compact ? undefined : t("live.mark")}
              </Button>
              <Menu
                label={t("live.more")}
                trigger={<Button icon="more_horiz" aria-label={t("live.more")} />}
                items={DISCARD_SECONDS.map((s) => ({ label: t("live.discard.menuItem", { count: s / 60 }), icon: "delete" as const, danger: true, onSelect: () => setDiscard(s) }))}
              />
            </>
          )}
          <HeaderRecord />
        </div>
      }
    >
      {active && session && <LiveToolbar meeting={meeting} mode={(session.mode as RecordMode) ?? pendingMode} compact={compact} />}
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-1">
        <Health />
        <Levels />
      </div>
      <LiveBanners />
      {active && discard != null && <DiscardPanel meeting={meeting} seconds={discard} onClose={() => setDiscard(null)} />}
      {active && marks > 0 && <p className="text-small m-0 text-muted">{t("live.markedCount", { count: marks })}</p>}
      <SpeakerStrip defaultOpen={!compact} />
      {layout === "transcript" || !active ? (
        <div className="grid min-h-0 flex-1 gap-4 [grid-template-columns:minmax(0,1fr)] min-[1100px]:[grid-template-columns:minmax(0,1fr)_340px]">
          <TranscriptView />
          {/* Beside the transcript at full width; under it (shorter) in the compact window. */}
          {active && <Notepad meeting={meeting} className="max-h-56 min-[1100px]:max-h-none" />}
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col gap-3">
          <FocusCaption />
          <Notepad meeting={meeting!} large />
        </div>
      )}
    </Page>
  );
}
