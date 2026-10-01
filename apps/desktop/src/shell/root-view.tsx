// SPDX-License-Identifier: Apache-2.0
// Above every route: core events feed the live store, and first launch goes
// to onboarding until it is done.
import { Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import type { CoreEvent } from "../bindings";
import { ipc } from "../ipc";
import { useLive } from "../state/live";
import { QuitDialog } from "./quit-dialog";

export const settingsQuery = {
  queryKey: ["settings"] as const,
  queryFn: async () => {
    const r = await ipc.commands.getSettings();
    if (r.status === "error") throw new Error(r.error);
    return r.data;
  },
};

export function RootView() {
  const navigate = useNavigate();
  const path = useRouterState({ select: (s) => s.location.pathname });
  const { data: settings } = useQuery(settingsQuery);
  const apply = useLive((s) => s.apply);
  // Core events → live store for every route (onboarding's test recording
  // included). A reloaded webview first restores the recording in progress (events it
  // already has are skipped by `seq`).
  const restore = useLive((s) => s.restore);
  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    // Listen first, then read the snapshot: nothing can fall between them.
    // Events that arrive before the snapshot wait, then follow it.
    let pending: CoreEvent[] | null = [];
    void (async () => {
      const u = await ipc.onCoreEvent((e) => (pending ? pending.push(e) : apply(e)));
      if (!alive) return u();
      off = u;
      try {
        const r = await ipc.commands.sessionSnapshot();
        if (alive && r.status === "ok" && r.data) restore(r.data);
      } finally {
        const queued = pending ?? [];
        pending = null;
        queued.forEach(apply);
      }
    })();
    return () => {
      alive = false;
      off?.();
    };
  }, [apply, restore]);

  useEffect(() => {
    if (settings && !settings.onboardingDone && !path.startsWith("/onboarding")) {
      void navigate({ to: "/onboarding/$step", params: { step: "welcome" } });
    }
  }, [settings, path, navigate]);
  return (
    <>
      <Outlet />
      {/* Here, not in the shell: quitting must ask during onboarding's test too. */}
      <QuitDialog />
    </>
  );
}
