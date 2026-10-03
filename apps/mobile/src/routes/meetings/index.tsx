// SPDX-License-Identifier: Apache-2.0
// M3 meetings + M4 meeting view routes (owned by 16-I). router.tsx mounts what this file returns; add
// child routes here, never in router.tsx.
import { createRoute, type AnyRoute } from "@tanstack/react-router";
import { Placeholder } from "../placeholder";

export function meetingsRoutes(parent: AnyRoute) {
  return createRoute({ getParentRoute: () => parent, path: "/meetings", component: () => <Placeholder screen="meetings" /> });
}
