// SPDX-License-Identifier: Apache-2.0
// Live meeting (D4): header (title, mode, layout, clock and record control),
// speaker strip and lanes, the virtualized transcript with the notepad beside
// it (Transcript layout) or the transcript in a narrow column beside the notes
// (Focus layout), health and levels in the footer, system states as inline
// banners. Pieces live in `features/live`.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { cn, usePlatform } from "@ghi/ui";
import type { RecordMode } from "../bindings";
import { Page } from "../shell/page";
import { useCompact } from "../shell/use-compact";
import { ConsentBanner, DiscardPanel, HeaderRecord, LiveBanners, LiveFooter, LiveHeader, LiveSystemBanners, Notepad, PausedOverlay, SpeakerStrip, TranscriptView, type LiveLayout } from "../features/live";
import { isActive, useLive } from "../state/live";
import { useUi } from "../state/ui";

const DISCARD_SECONDS = [60, 300, 600];

export function LiveScreen() {
  const { t } = useTranslation();
  const platform = usePlatform();
  const compact = useCompact();
  const state = useLive((s) => s.state);
  const meeting = useLive((s) => s.meeting);
  const session = useLive((s) => s.session);
  const pendingMode = useUi((s) => s.recordMode);
  const [layout, setLayout] = useState<LiveLayout>("transcript");
  const [discard, setDiscard] = useState<number | null>(null);
  const active = isActive(state) && meeting != null;

  if (!active) {
    return (
      <Page title={t("nav.live")} subtitle={compact ? undefined : t("live.localLine", { context: platform })} className="flex min-h-0 flex-col gap-3 overflow-hidden" actions={<HeaderRecord />}>
        <LiveBanners />
        <SpeakerStrip />
        <TranscriptView />
      </Page>
    );
  }

  const mode = (session?.mode as RecordMode | undefined) ?? pendingMode;
  const focus = layout === "focus";
  const paused = state === "paused";
  return (
    <div className="flex h-full min-h-0 flex-col">
      <LiveSystemBanners />
      <LiveHeader meeting={meeting} mode={mode} layout={layout} onLayout={setLayout} onDiscard={setDiscard} discardSeconds={DISCARD_SECONDS} compact={compact} />
      <SpeakerStrip />
      {discard != null && (
        <div className="px-5 pb-2">
          <DiscardPanel meeting={meeting} seconds={discard} onClose={() => setDiscard(null)} />
        </div>
      )}
      <div
        className={cn(
          "relative grid min-h-0 flex-1 gap-5 px-5 pb-3.5",
          focus ? (compact ? "grid-cols-[minmax(0,1fr)_240px]" : "grid-cols-[minmax(0,1fr)_340px]") : compact ? "grid-cols-[minmax(0,1fr)_260px]" : "grid-cols-[minmax(0,1.5fr)_minmax(0,1fr)]",
        )}
      >
        <div inert={paused} className={cn("flex min-h-0 flex-col gap-2", focus && "order-2")}>
          <ConsentBanner meeting={meeting} />
          <LiveBanners />
          <TranscriptView small={focus} />
        </div>
        <Notepad meeting={meeting} inert={paused} className={focus ? "order-1" : undefined} />
        {paused && <PausedOverlay />}
      </div>
      <LiveFooter room={mode === "room"} />
    </div>
  );
}
