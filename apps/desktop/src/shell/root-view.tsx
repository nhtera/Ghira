// SPDX-License-Identifier: Apache-2.0
// Above every route: core events feed the live store. In the main window,
// first launch goes to onboarding until it is done, and quitting while
// recording asks. The small panels (popover, mini-recorder, detection prompt)
// get neither: they are separate windows with their own few commands.
import { Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import type { CoreEvent } from "../bindings";
import { ipc } from "../ipc";
import { useLive } from "../state/live";
import { QuitDialog } from "./quit-dialog";
import { LockGate } from "./lock-gate";
import { useImportListeners } from "../features/import/import-store";
import { MassDeleteDialog } from "../features/sync/mass-delete-dialog";

/** Main window only: files dropped on the window or the Dock (D10) and import
 * progress are kept even before the import screen opens. */
function ImportListeners() {
  useImportListeners();
  return null;
}

export const settingsQuery = {
  queryKey: ["settings"] as const,
  queryFn: async () => {
    const r = await ipc.commands.getSettings();
    if (r.status === "error") throw new Error(r.error);
    return r.data;
  },
};

const PANEL_ROUTES = ["/popover", "/mini", "/detect"];

export function RootView() {
  const navigate = useNavigate();
  const path = useRouterState({ select: (s) => s.location.pathname });
  const panel = PANEL_ROUTES.some((p) => path.startsWith(p));
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

  // The core sends no transcript while locked: read the recording again on unlock.
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void ipc
      .onLockChanged(async (e) => {
        if (e.locked) return;
        const r = await ipc.commands.sessionSnapshot();
        if (!gone && r.status === "ok" && r.data) restore(r.data);
      })
      .then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  }, [restore]);

  useEffect(() => {
    if (!panel && settings && !settings.onboardingDone && !path.startsWith("/onboarding")) {
      void navigate({ to: "/onboarding/$step", params: { step: "welcome" } });
    }
  }, [panel, settings, path, navigate]);
  return (
    <>
      <LockGate mode={path.startsWith("/mini") || path.startsWith("/detect") ? "controls" : "full"}>
        <Outlet />
      </LockGate>
      {/* Here, not in the shell: quitting must ask during onboarding's test too. */}
      {!panel && <QuitDialog />}
      {!panel && <ImportListeners />}
      {/* Here, not in Settings: another device's mass delete asks wherever the user is. */}
      {!panel && <MassDeleteDialog />}
    </>
  );
}
