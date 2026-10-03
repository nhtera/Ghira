// SPDX-License-Identifier: Apache-2.0
// M2 record routes (owned by 16-H). router.tsx mounts what this file returns; add
// child routes here, never in router.tsx.
import { createRoute, type AnyRoute } from "@tanstack/react-router";
import { Placeholder } from "../placeholder";

export function recordRoutes(parent: AnyRoute) {
  return createRoute({ getParentRoute: () => parent, path: "/record", component: () => <Placeholder screen="record" /> });
}
