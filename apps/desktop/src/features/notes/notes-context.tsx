// SPDX-License-Identifier: Apache-2.0
// What every block of one meeting's notes needs: the meeting, its speakers,
// whether audio is left (for citation chips) and the edit commands.
import { createContext, useContext } from "react";
import type { MarkedMoment, MeetingSpeaker } from "../../bindings";
import type { NotesEdit } from "./use-notes-edit";

export type NotesContextValue = {
  meeting: string;
  speakers: readonly MeetingSpeaker[];
  audioAvailable: boolean;
  /** The moments marked while recording, with what covers each. */
  marks?: readonly MarkedMoment[];
  edit: NotesEdit;
};

export const NotesContext = createContext<NotesContextValue | null>(null);

export function useNotesContext(): NotesContextValue {
  const v = useContext(NotesContext);
  if (!v) throw new Error("NotesContext missing");
  return v;
}
