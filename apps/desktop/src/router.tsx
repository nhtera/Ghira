// SPDX-License-Identifier: Apache-2.0
// Routes (phase 9 plan): code-based, hash history (the app is served from a
// custom scheme; a reload must never ask it for a path that isn't a file).
import {
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
  type RouterHistory,
} from "@tanstack/react-router";
import { AppShell } from "./shell/app-shell";
import { RootView } from "./shell/root-view";
import { LiveScreen } from "./routes/live";
import { MeetingsScreen } from "./routes/meetings";
import { OnboardingScreen } from "./routes/onboarding";
import { AskScreen, ImportScreen, MeetingDetailScreen, PeopleScreen } from "./routes/simple";
import { SETTINGS_SECTIONS, SettingsScreen, type SettingsSection } from "./routes/settings";

// Root: global listeners only. The app shell (sidebar, title bar) is a
// layout route; onboarding renders full-window outside it.
const root = createRootRoute({ component: RootView });

const shell = createRoute({ getParentRoute: () => root, id: "shell", component: AppShell });

const index = createRoute({
  getParentRoute: () => shell,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: "/meetings" });
  },
});

const meetings = createRoute({ getParentRoute: () => shell, path: "/meetings", component: MeetingsScreen });

const meetingDetail = createRoute({
  getParentRoute: () => shell,
  path: "/meetings/$id/$tab",
  params: {
    parse: (p) => ({ id: p.id, tab: p.tab === "transcript" ? ("transcript" as const) : ("notes" as const) }),
    stringify: (p) => ({ id: p.id, tab: p.tab }),
  },
  component: MeetingDetailScreen,
});

const live = createRoute({ getParentRoute: () => shell, path: "/live", component: LiveScreen });
const people = createRoute({ getParentRoute: () => shell, path: "/people", component: PeopleScreen });
const ask = createRoute({ getParentRoute: () => shell, path: "/ask", component: AskScreen });
const importRoute = createRoute({ getParentRoute: () => shell, path: "/import", component: ImportScreen });

const settings = createRoute({
  getParentRoute: () => shell,
  path: "/settings/$section",
  params: {
    parse: (p) => ({
      section: (SETTINGS_SECTIONS as readonly string[]).includes(p.section) ? (p.section as SettingsSection) : "general",
    }),
    stringify: (p) => ({ section: p.section }),
  },
  component: SettingsScreen,
});

const onboarding = createRoute({
  getParentRoute: () => root,
  path: "/onboarding/$step",
  component: OnboardingScreen,
});

const routeTree = root.addChildren([
  shell.addChildren([index, meetings, meetingDetail, live, people, ask, importRoute, settings]),
  onboarding,
]);

export function makeRouter(history: RouterHistory = createHashHistory()) {
  return createRouter({ routeTree, history, defaultPreload: false });
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof makeRouter>;
  }
}
