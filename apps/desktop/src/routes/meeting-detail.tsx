// SPDX-License-Identifier: Apache-2.0
// Meeting detail (D6): notes, transcript and the audio bar (phase 11b).
import { useParams, useSearch } from "@tanstack/react-router";
import { MeetingScreen } from "../features/meeting/meeting-screen";

export function MeetingDetailScreen() {
  const { id, tab } = useParams({ from: "/shell/meetings/$id/$tab" });
  const { t } = useSearch({ from: "/shell/meetings/$id/$tab" });
  return <MeetingScreen id={id} tab={tab} startAtMs={t} />;
}
