// SPDX-License-Identifier: Apache-2.0
// M3 meetings + M4 meeting view routes (owned by 16-I). router.tsx mounts what this file returns; add
// child routes here, never in router.tsx.
// /meetings is the list; /meetings/$id is a meeting (?tab=notes|actions|transcript, ?at=<ms> to open a line).
import {
  createRoute,
  Outlet,
  redirect,
  type AnyRoute,
} from "@tanstack/react-router";
import { needsOnboarding } from "../../features/onboarding/state";
import { MeetingList } from "../../features/meeting-list/meeting-list";
import {
  MeetingView,
  type MeetingTab,
} from "../../features/meeting-view/meeting-view";

const TABS: readonly string[] = ["notes", "actions", "transcript"];

export function meetingsRoutes(parent: AnyRoute) {
  const meetings = createRoute({
    getParentRoute: () => parent,
    path: "/meetings",
    beforeLoad: async () => {
      if (await needsOnboarding()) throw redirect({ to: "/onboarding" });
    },
    component: Outlet,
  });
  const list = createRoute({
    getParentRoute: () => meetings,
    path: "/",
    component: MeetingList,
  });
  const view = createRoute({
    getParentRoute: () => meetings,
    path: "$id",
    validateSearch: (
      s: Record<string, unknown>,
    ): { tab?: MeetingTab; at?: number } => ({
      tab:
        typeof s.tab === "string" && TABS.includes(s.tab)
          ? (s.tab as MeetingTab)
          : undefined,
      at: typeof s.at === "number" && Number.isFinite(s.at) ? s.at : undefined,
    }),
    component: function MeetingRoute() {
      const { id } = view.useParams();
      const { tab, at } = view.useSearch();
      // Keyed, so another meeting starts with its own tab, audio and sheets.
      return <MeetingView key={id} id={id} tab={tab} at={at} />;
    },
  });
  return meetings.addChildren([list, view]);
}
