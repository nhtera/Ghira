// SPDX-License-Identifier: Apache-2.0
// Meetings being processed right now, from `jobProgress` / `stateChanged` /
// `notesReady` core events. Lives outside React so a toast can fire from any
// screen and the library can still show the stepper after a detour.
import { create } from "zustand";
import type { Event, Stage } from "../../bindings";

export type Processing = { stage: Stage | null; kind: string; progress: number | null };

type State = {
  meetings: Record<string, Processing>;
  /** Meetings whose notes finished this session (offers "Name your speakers"). */
  finished: string[];
  apply: (e: Event) => void;
  clearFinished: (meeting: string) => void;
  /** The meeting is gone (deleted): drop everything about it. */
  forget: (meeting: string) => void;
};

export const STAGES: Stage[] = ["decoding", "refiningSpeakers", "matchingVoices", "improvingTranscript", "writingNotes"];

/** `notes_live` / `notes_final` jobs carry no stage: they are the writing step. */
export function stageOf(kind: string, stage: Stage | null): Stage | null {
  if (stage) return stage;
  return kind === "notes_live" || kind === "notes_final" ? "writingNotes" : null;
}

const without = <T,>(o: Record<string, T>, k: string) => {
  const rest = { ...o };
  delete rest[k];
  return rest;
};

export const useProcessing = create<State>((set) => ({
  meetings: {},
  finished: [],
  apply: (e) =>
    set((s) => {
      switch (e.type) {
        case "jobProgress":
          if (!e.meeting) return s;
          return { meetings: { ...s.meetings, [e.meeting]: { stage: stageOf(e.kind, e.stage), kind: e.kind, progress: e.progress } } };
        case "stateChanged":
          if (e.state === "processing") return s.meetings[e.meeting] ? s : { meetings: { ...s.meetings, [e.meeting]: { stage: null, kind: "final_pass", progress: null } } };
          if (e.state === "ready" || e.state === "failed" || e.state === "idle") return s.meetings[e.meeting] ? { meetings: without(s.meetings, e.meeting) } : s;
          return s;
        case "error":
          // A failed job must not leave its stepper spinning.
          return e.kind === "job" && e.meeting && s.meetings[e.meeting] ? { meetings: without(s.meetings, e.meeting) } : s;
        case "notesReady":
          // v1 (live notes) is followed by a final pass that re-diarizes: names are asked after v2.
          return {
            meetings: without(s.meetings, e.meeting),
            finished: e.version < 2 || s.finished.includes(e.meeting) ? s.finished : [...s.finished, e.meeting],
          };
        default:
          return s;
      }
    }),
  forget: (meeting) => set((s) => ({ meetings: without(s.meetings, meeting), finished: s.finished.filter((m) => m !== meeting) })),
  clearFinished: (meeting) => set((s) => ({ finished: s.finished.filter((m) => m !== meeting) })),
}));
