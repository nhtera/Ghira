// SPDX-License-Identifier: Apache-2.0
// Search routes (owned by 16-I). router.tsx mounts what this file returns; add
// child routes here, never in router.tsx.
import { createRoute, type AnyRoute } from "@tanstack/react-router";
import { Placeholder } from "../placeholder";

export function searchRoutes(parent: AnyRoute) {
  return createRoute({ getParentRoute: () => parent, path: "/search", component: () => <Placeholder screen="search" /> });
}
