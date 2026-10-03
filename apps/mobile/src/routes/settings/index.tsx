// SPDX-License-Identifier: Apache-2.0
// Settings, privacy, app lock, cloud sheet, M5 inbox routes (owned by 16-J). router.tsx mounts what this file returns; add
// child routes here, never in router.tsx.
import { createRoute, type AnyRoute } from "@tanstack/react-router";
import { Placeholder } from "../placeholder";

export function settingsRoutes(parent: AnyRoute) {
  return createRoute({ getParentRoute: () => parent, path: "/settings", component: () => <Placeholder screen="settings" /> });
}
