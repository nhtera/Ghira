// SPDX-License-Identifier: Apache-2.0
// Search hits grouped by meeting, in the order each meeting first appears
// (the store ranks exact accented matches first, so those meetings lead).
import type { SearchHitView } from "../../bindings";

export type HitGroup = {
  meeting: string;
  title: string;
  startedAt: number | null;
  hits: SearchHitView[];
};

export function groupHits(hits: readonly SearchHitView[]): HitGroup[] {
  const byMeeting = new Map<string, HitGroup>();
  for (const h of hits) {
    let g = byMeeting.get(h.meeting);
    if (!g)
      byMeeting.set(
        h.meeting,
        (g = {
          meeting: h.meeting,
          title: h.meetingTitle,
          startedAt: h.meetingStartedAt,
          hits: [],
        }),
      );
    g.hits.push(h);
  }
  return [...byMeeting.values()];
}
