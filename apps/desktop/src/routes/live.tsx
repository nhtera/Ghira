// SPDX-License-Identifier: Apache-2.0
// Live meeting (D4). Phase 9: record control, levels, speakers and the
// transcript on the live store, plus the record-only state; the full layouts
// (lanes, notes pad, health) come in phase 10.
import { memo, useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { useTranslation } from "react-i18next";
import { Button, LevelMeter, RecordControl, SpeakerChip, TranscriptLine, usePlatform, wordsFromText, type RecordState } from "@ghi/ui";
import type { LineInfo, SessionState, SpeakerInfo } from "../bindings";
import { useAppActions } from "../shell/actions";
import { Page } from "../shell/page";
import { elapsedMs, useLive } from "../state/live";
import { speakerNumber, useSpeakerLabel } from "../state/speaker-label";
import { useUi } from "../state/ui";

function recordState(s: SessionState): RecordState {
  switch (s) {
    case "starting":
    case "recording":
    case "paused":
    case "stopping":
      return s;
    case "failed":
      return "error";
    default:
      return "idle";
  }
}

/** Ticks once a second while recording (the record control's clock). */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [active]);
  return now;
}

const chipState = (s: SpeakerInfo) => (s.provisional ? "identifying" : speakerNumber(s) ? "numbered" : "named");

type Speaker = ReturnType<typeof toSpeaker>;
const toSpeaker = (label: string, s: SpeakerInfo) => ({ label, colorSlot: s.colorSlot, isMe: s.isMe, initial: speakerNumber(s) ?? undefined });

/** One final line; unchanged lines don't re-render as new ones arrive. */
const Row = memo(
  function Row({ line, speaker, marked }: { line: LineInfo; speaker: Speaker | null; marked: boolean }) {
    return (
      <li>
        <TranscriptLine
          startMs={line.t0Ms ?? 0}
          speaker={speaker}
          words={line.words.length ? line.words.map((w) => ({ text: w.text, lowConfidence: w.lowConfidence })) : wordsFromText(line.text)}
          marked={marked}
        />
      </li>
    );
  },
  (a, b) => a.line === b.line && a.marked === b.marked && a.speaker?.label === b.speaker?.label && a.speaker?.colorSlot === b.speaker?.colorSlot,
);

export function LiveScreen() {
  const { t } = useTranslation();
  const platform = usePlatform();
  // Per-field selectors: a level or partial event must not re-render every line.
  const state = useLive((s) => s.state);
  const lines = useLive((s) => s.lines);
  const partial = useLive((s) => s.partial);
  const speakers = useLive((s) => s.speakers);
  const recordOnly = useLive((s) => s.recordOnly);
  const marks = useLive((s) => s.marks);
  const levels = useLive((s) => s.levels);
  const clock = useLive(useShallow((s) => ({ startedAtMs: s.startedAtMs, pausedAtMs: s.pausedAtMs, pausedTotalMs: s.pausedTotalMs })));
  const { startRecording, stopRecording, pauseRecording, resumeRecording, mark } = useAppActions();
  const mode = useUi((s) => s.recordMode);
  const setMode = useUi((s) => s.setRecordMode);
  const now = useNow(state === "recording");
  const words = Object.values(partial).filter(Boolean).join(" ");
  const recording = state === "recording" || state === "paused";
  const labelOf = useSpeakerLabel();
  const speakerOf = (id: number | null) => {
    const s = id != null ? speakers[id] : undefined;
    return s ? toSpeaker(labelOf(s), s) : null;
  };
  return (
    <Page
      title={t("nav.live")}
      subtitle={t("live.localLine", { context: platform })}
      actions={
        <div className="flex items-center gap-2">
          {recording && (
            <Button icon="star" onClick={() => void mark()}>
              {t("live.mark")}
            </Button>
          )}
          <RecordControl
            state={recordState(state)}
            mode={mode}
            elapsedMs={elapsedMs(clock, now)}
            onStart={(m) => void startRecording(m)}
            onModeChange={setMode}
            onPause={() => void pauseRecording()}
            onResume={() => void resumeRecording()}
            onStop={() => void stopRecording()}
          />
        </div>
      }
    >
      <div className="mb-4 grid max-w-xl grid-cols-2 gap-4">
        <LevelMeter source="mic" db={levels.mic} />
        <LevelMeter source="system" db={levels.system} />
      </div>
      {Object.keys(speakers).length > 0 && (
        <ul aria-label={t("speakers.title")} className="m-0 mb-4 flex list-none flex-wrap gap-1.5 p-0">
          {Object.values(speakers).map((s) => (
            <li key={s.id}>
              <SpeakerChip state={chipState(s)} name={labelOf(s)} colorSlot={s.colorSlot} isMe={s.isMe} />
            </li>
          ))}
        </ul>
      )}
      {recordOnly && (
        <p role="status" className="text-body m-0 mb-4 rounded-row bg-warn-soft px-4 py-2.5 text-warn">
          {t("live.deferredBanner")}
        </p>
      )}
      {state === "paused" && (
        <p role="status" className="text-body m-0 mb-4">
          {t("live.paused.title")} <span className="text-muted">{t("live.paused.subtitle")}</span>
        </p>
      )}
      {marks.length > 0 && <p className="text-small m-0 mb-2 text-muted">{t("live.markedCount", { count: marks.length })}</p>}
      {/* Announcing every line is too noisy: the shell announces new speaker turns. */}
      <ol aria-live="off" className="m-0 flex list-none flex-col gap-1 p-0">
        {lines.map((l) => (
          <Row key={l.gid || l.t0Ms} line={l} speaker={speakerOf(l.speaker)} marked={marks.some((m) => m >= (l.t0Ms ?? 0) && m <= (l.t1Ms ?? 0))} />
        ))}
        {words && (
          <li>
            <TranscriptLine startMs={lines.at(-1)?.t1Ms ?? 0} speaker={null} words={wordsFromText(words)} partial />
          </li>
        )}
      </ol>
    </Page>
  );
}
