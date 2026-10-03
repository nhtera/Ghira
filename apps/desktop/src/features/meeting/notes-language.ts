// SPDX-License-Identifier: Apache-2.0
// Which language a language switch shows as pressed: the one picked here, else
// the language the notes were written in, which is what Settings → Languages
// says (`notesLanguage`) or, for "as spoken", the transcript's language. Never
// a guess from the interface language: when it is unknown nothing is pressed.
import { useQuery } from "@tanstack/react-query";
import type { MeetingDetail, NotesLanguage } from "../../bindings";
import { settingsQuery } from "../../shell/root-view";

export type PickedLanguage = "en" | "vi";

const known = (l: string | null | undefined): PickedLanguage | null => (l === "en" || l === "vi" ? l : null);

export function useNotesLanguage(detail: Pick<MeetingDetail, "language"> | undefined, picked: PickedLanguage | null) {
  const setting = useQuery(settingsQuery).data?.notesLanguage;
  const written = known(setting) ?? known(detail?.language);
  /** The switch's pressed side, if known. */
  const shown = picked ?? written;
  /** What to ask the core for: the explicit pick, else the meeting's own. */
  const request: NotesLanguage = picked ?? "meeting";
  return { shown, request };
}
