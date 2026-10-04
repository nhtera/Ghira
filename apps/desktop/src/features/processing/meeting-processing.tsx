// SPDX-License-Identifier: Apache-2.0
// D5 on the meeting's own page (inside the Notes tab, where the notes will appear): the stepper while its notes are written, then
// "Name your speakers". After Stop the app opens the new meeting here; the
// library shows the same panels for meetings processed in the background.
import { useEffect } from "react";
import type { MeetingJob } from "../../bindings";
import { NameSpeakers } from "./name-speakers";
import { ProcessingPanel } from "./processing-panel";
import { useProcessing } from "./processing-store";
import { useUnnamed } from "./use-unnamed";

/**
 * `job`: what the core says is running. After a relaunch, or while the models are missing, no event
 * arrived in this session, so the stepper is built from it.
 */
export function MeetingProcessing({ meeting, waitingForModels, job }: { meeting: string; waitingForModels?: boolean; job?: MeetingJob | null }) {
  const stored = useProcessing((s) => s.meetings[meeting]);
  const processing = stored ?? (job ? { stage: null, kind: job.kind, progress: job.progress } : undefined);
  waitingForModels ??= job?.waitingForModels;
  const finished = useProcessing((s) => s.finished.includes(meeting));
  const clearFinished = useProcessing((s) => s.clearFinished);
  const { left, loaded, markNamed } = useUnnamed(finished ? meeting : undefined);
  useEffect(() => {
    if (finished && loaded && left.length === 0) clearFinished(meeting);
  }, [finished, loaded, left.length, clearFinished, meeting]);
  return (
    <div>
      {processing && <ProcessingPanel processing={processing} waitingForModels={waitingForModels} />}
      {finished && left.length > 0 && <NameSpeakers meeting={meeting} speakers={left} onDone={markNamed} onSkipAll={() => clearFinished(meeting)} />}
    </div>
  );
}
