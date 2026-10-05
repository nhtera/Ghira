// SPDX-License-Identifier: Apache-2.0
// Routes: code-based, hash history (the app is served from a custom scheme).
// A tab shell (Meetings / Record / Search / Settings) holds the main screens;
// onboarding renders full-screen outside it. Screens: routes/<area>/ (16-H..J).
import {
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
  useRouter,
  type RouterHistory,
} from "@tanstack/react-router";
import { LoadFailed, PageNotFound } from "./features/store-problem";
import { RootView } from "./shell/root-view";
import { TabShell } from "./shell/tab-shell";
import { meetingsRoutes } from "./routes/meetings";
import { onboardingRoutes } from "./routes/onboarding";
import { recordRoutes } from "./routes/record";
import { searchRoutes } from "./routes/search";
import { settingsRoutes } from "./routes/settings";

const root = createRootRoute({ component: RootView });

const shell = createRoute({ getParentRoute: () => root, id: "shell", component: TabShell });

const index = createRoute({
  getParentRoute: () => shell,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: "/meetings" });
  },
});

// Each area builds its own routes (routes/<area>/index.tsx); only the
// mount points live here.
const meetings = meetingsRoutes(shell);
const record = recordRoutes(shell);
const search = searchRoutes(shell);
const settings = settingsRoutes(shell);

// Outside the shell: first launch (M1).
const onboarding = onboardingRoutes(root);

const routeTree = root.addChildren([shell.addChildren([index, meetings, record, search, settings]), onboarding]);

// A route that throws (a loader, a render) shows "couldn't load" with "Try
// again"; one that doesn't exist says so and goes home. Never a blank page.
function RouteFailed({ reset }: { reset?: () => void }) {
  const router = useRouter();
  return (
    <LoadFailed
      onRetry={async () => {
        await router.invalidate();
        reset?.();
      }}
    />
  );
}

function RouteNotFound() {
  const router = useRouter();
  return <PageNotFound onHome={() => void router.navigate({ to: "/meetings", replace: true })} />;
}

export function makeRouter(history?: RouterHistory) {
  return createRouter({
    routeTree,
    history: history ?? createHashHistory(),
    defaultPreload: false,
    defaultErrorComponent: RouteFailed,
    defaultNotFoundComponent: RouteNotFound,
  });
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof makeRouter>;
  }
}
