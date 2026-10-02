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
import { DetectScreen } from "./routes/detect";
import { MiniScreen } from "./routes/mini";
import { OnboardingScreen } from "./routes/onboarding";
import { PopoverScreen } from "./routes/popover";
import { AskScreen } from "./routes/ask";
import { PeopleScreen } from "./routes/people";
import { ImportScreen } from "./routes/import";
import { MeetingDetailScreen } from "./routes/meeting-detail";
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

const meetings = createRoute({
  getParentRoute: () => shell,
  path: "/meetings",
  // `q`: start with this text in the library search (from Ask).
  validateSearch: (s: Record<string, unknown>): { q?: string } => (typeof s.q === "string" && s.q ? { q: s.q } : {}),
  component: MeetingsScreen,
});

const meetingDetail = createRoute({
  getParentRoute: () => shell,
  path: "/meetings/$id/$tab",
  params: {
    parse: (p) => ({ id: p.id, tab: p.tab === "transcript" ? ("transcript" as const) : ("notes" as const) }),
    stringify: (p) => ({ id: p.id, tab: p.tab }),
  },
  // `t`: open at this meeting time (a search hit), in ms.
  validateSearch: (s: Record<string, unknown>): { t?: number } => {
    const t = Number(s.t);
    return Number.isFinite(t) && t >= 0 ? { t } : {};
  },
  component: MeetingDetailScreen,
});

const live = createRoute({ getParentRoute: () => shell, path: "/live", component: LiveScreen });
const people = createRoute({ getParentRoute: () => shell, path: "/people", component: PeopleScreen });
const ask = createRoute({
  getParentRoute: () => shell,
  path: "/ask",
  // `meeting`: adds the "This meeting" scope.
  validateSearch: (s: Record<string, unknown>): { meeting?: string } => (typeof s.meeting === "string" && s.meeting ? { meeting: s.meeting } : {}),
  component: AskScreen,
});
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

// The menu-bar popover and the mini-recorder windows (outside the shell).
const popover = createRoute({ getParentRoute: () => root, path: "/popover", component: PopoverScreen });
const mini = createRoute({ getParentRoute: () => root, path: "/mini", component: MiniScreen });
const detect = createRoute({ getParentRoute: () => root, path: "/detect", component: DetectScreen });

const routeTree = root.addChildren([
  shell.addChildren([index, meetings, meetingDetail, live, people, ask, importRoute, settings]),
  onboarding,
  popover,
  mini,
  detect,
]);

export function makeRouter(history: RouterHistory = createHashHistory()) {
  return createRouter({ routeTree, history, defaultPreload: false });
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof makeRouter>;
  }
}
