// SPDX-License-Identifier: Apache-2.0
// Import (D10) rules, kept out of the components: what can be imported, the
// choice sent to the core, and the queue's per-file state machine.
import type { ImportChoice, ImportState, ImportUpdate, StagedFile } from "../../bindings";

/** Unsupported and empty files can't be imported; a duplicate is skipped until it's removed (the core refuses it). */
export const BLOCKING = ["unsupported", "empty", "duplicate"] as const;
export const isImportable = (f: StagedFile) => !f.problems.some((p) => (BLOCKING as readonly string[]).includes(p));

export type Language = "auto" | "en" | "vi";

export function importChoice(language: Language, splitChannels: boolean, files: readonly StagedFile[]): ImportChoice {
  // "Split channels" only means something when a file has two channels.
  return {
    language: language === "auto" ? null : language,
    splitChannels: splitChannels && files.some((f) => f.channels >= 2),
  };
}

export type QueueItem = {
  id: string;
  name: string;
  state: ImportState;
  progress: number | null;
  meeting: string | null;
  error: string | null;
};
export type Queue = Record<string, QueueItem>;

export type QueueAction = { type: "start"; files: { id: string; name: string }[] } | { type: "update"; update: ImportUpdate } | { type: "clear" };

const TERMINAL: ImportState[] = ["done", "failed", "cancelled"];
export const isActive = (i: QueueItem) => !TERMINAL.includes(i.state);

export function queueReducer(q: Queue, a: QueueAction): Queue {
  switch (a.type) {
    case "start":
      // start_import emits its first updates before it returns: keep whatever already arrived, only fix the name.
      return {
        ...q,
        ...Object.fromEntries(
          a.files.map((f) => [
            f.id,
            q[f.id]
              ? { ...q[f.id]!, name: f.name }
              : ({ id: f.id, name: f.name, state: "queued", progress: null, meeting: null, error: null } satisfies QueueItem),
          ]),
        ),
      };
    case "update": {
      const u = a.update;
      const cur = q[u.id];
      // A late "decoding" tick must not undo a finished file.
      if (cur && !isActive(cur)) return q;
      return {
        ...q,
        [u.id]: {
          id: u.id,
          name: cur?.name ?? u.id,
          state: u.state,
          progress: u.progress ?? cur?.progress ?? null,
          meeting: u.meeting ?? cur?.meeting ?? null,
          error: u.error,
        },
      };
    }
    case "clear":
      return Object.fromEntries(Object.entries(q).filter(([, i]) => isActive(i)));
  }
}

export const activeCount = (q: Queue) => Object.values(q).filter(isActive).length;
export const doneCount = (q: Queue) => Object.values(q).filter((i) => i.state === "done").length;
