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
import { LiveScreen } from "./routes/live";
import { MeetingsScreen } from "./routes/meetings";
import { AskScreen, ImportScreen, MeetingDetailScreen, OnboardingScreen, PeopleScreen } from "./routes/simple";
import { SETTINGS_SECTIONS, SettingsScreen, type SettingsSection } from "./routes/settings";

const root = createRootRoute({ component: AppShell });

const index = createRoute({
  getParentRoute: () => root,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: "/meetings" });
  },
});

const meetings = createRoute({ getParentRoute: () => root, path: "/meetings", component: MeetingsScreen });

const meetingDetail = createRoute({
  getParentRoute: () => root,
  path: "/meetings/$id/$tab",
  params: {
    parse: (p) => ({ id: p.id, tab: p.tab === "transcript" ? ("transcript" as const) : ("notes" as const) }),
    stringify: (p) => ({ id: p.id, tab: p.tab }),
  },
  component: MeetingDetailScreen,
});

const live = createRoute({ getParentRoute: () => root, path: "/live", component: LiveScreen });
const people = createRoute({ getParentRoute: () => root, path: "/people", component: PeopleScreen });
const ask = createRoute({ getParentRoute: () => root, path: "/ask", component: AskScreen });
const importRoute = createRoute({ getParentRoute: () => root, path: "/import", component: ImportScreen });

const settings = createRoute({
  getParentRoute: () => root,
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

const routeTree = root.addChildren([index, meetings, meetingDetail, live, people, ask, importRoute, settings, onboarding]);

export function makeRouter(history: RouterHistory = createHashHistory()) {
  return createRouter({ routeTree, history, defaultPreload: false });
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof makeRouter>;
  }
}
