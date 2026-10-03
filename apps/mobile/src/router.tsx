// SPDX-License-Identifier: Apache-2.0
// Routes: code-based, hash history (the app is served from a custom scheme).
// A tab shell (Meetings / Record / Search / Settings) holds the main screens;
// onboarding renders full-screen outside it. Screens arrive with 16-H..J.
import {
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
  type RouterHistory,
} from "@tanstack/react-router";
import { RootView } from "./shell/root-view";
import { TabShell } from "./shell/tab-shell";
import { Placeholder } from "./routes/placeholder";

const root = createRootRoute({ component: RootView });

const shell = createRoute({ getParentRoute: () => root, id: "shell", component: TabShell });

const index = createRoute({
  getParentRoute: () => shell,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: "/meetings" });
  },
});

const tab = <P extends string>(path: P, screen: "meetings" | "record" | "search" | "settings") =>
  createRoute({ getParentRoute: () => shell, path, component: () => <Placeholder screen={screen} /> });

const meetings = tab("/meetings", "meetings");
const record = tab("/record", "record");
const search = tab("/search", "search");
const settings = tab("/settings", "settings");

// Outside the shell: first launch (M1).
const onboarding = createRoute({
  getParentRoute: () => root,
  path: "/onboarding",
  component: () => <Placeholder screen="onboarding" />,
});

const routeTree = root.addChildren([shell.addChildren([index, meetings, record, search, settings]), onboarding]);

export function makeRouter(history?: RouterHistory) {
  return createRouter({ routeTree, history: history ?? createHashHistory(), defaultPreload: false });
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof makeRouter>;
  }
}
