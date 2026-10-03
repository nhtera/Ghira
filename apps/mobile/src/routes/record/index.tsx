// SPDX-License-Identifier: Apache-2.0
// M2 record routes (owned by 16-H). router.tsx mounts what this file returns; add
// child routes here, never in router.tsx.
import { createRoute, redirect, type AnyRoute } from "@tanstack/react-router";
import { needsOnboarding } from "../../features/onboarding/state";
import { RecordScreen } from "../../features/record/record-screen";

export function recordRoutes(parent: AnyRoute) {
  return createRoute({
    getParentRoute: () => parent,
    path: "/record",
    // First launch: the steps come before the first recording.
    beforeLoad: async () => {
      if (await needsOnboarding()) throw redirect({ to: "/onboarding" });
    },
    component: RecordScreen,
  });
}
