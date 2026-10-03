// SPDX-License-Identifier: Apache-2.0
// The window: title bar, sidebar, the routed screen, the command palette.
// Wires core events into the live store, the keyboard map, and the macOS
// menu's actions.
import { Outlet, useNavigate } from "@tanstack/react-router";
import { UpdateBanner } from "../features/settings/update-banner";
import { useEffect, useRef } from "react";
import { usePlatform } from "@ghi/ui";
import { ipc } from "../ipc";
import { inTauri } from "../ipc/ipc";
import { useLive } from "../state/live";
import { useAppActions } from "./actions";
import { DetectionPrompt } from "../features/detection/detection-prompt";
import { ProcessingWatcher } from "../features/processing/processing-watcher";
import { SystemStates, UpdateReady } from "../features/system-states";
import { CommandPalette } from "./command-palette";
import { LiveAnnouncer } from "./live-announcer";
import { shortcutFor } from "./shortcuts";
import { Sidebar } from "./sidebar";
import { TitleBar } from "./title-bar";
import { useCompact } from "./use-compact";

export function AppShell() {
  const compact = useCompact();
  const platform = usePlatform();
  const { run } = useAppActions();
  const navigate = useNavigate();


  // Listeners subscribe once and call the latest `run` (re-subscribing on
  // every state change could drop a menu event in between).
  const runRef = useRef(run);
  useEffect(() => {
    runRef.current = run;
  }, [run]);

  // Rust brought the window forward for a route (tray, notifications).
  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    void ipc
      .onNavigate((e) => void navigate({ to: e.route as "/meetings" }))
      .then((u) => (alive ? (off = u) : u()));
    return () => {
      alive = false;
      off?.();
    };
  }, [navigate]);

  // The macOS menu (inside Tauri) sends the chords it owns.
  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    void ipc.onMenuAction((a) => runRef.current(a)).then((u) => (alive ? (off = u) : u()));
    return () => {
      alive = false;
      off?.();
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const id = shortcutFor(e, platform, inTauri());
      // A held chord repeats: act on the first press only.
      if (!id || e.repeat) return;
      if (runRef.current(id)) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [platform]);

  const state = useLive((s) => s.state);
  const meeting = useLive((s) => s.meeting);
  // Stopping lands on the new meeting (its processing banner), not the library.
  useEffect(() => {
    if (state !== "processing") return;
    if (meeting) void navigate({ to: "/meetings/$id/$tab", params: { id: meeting, tab: "notes" } });
    else void navigate({ to: "/meetings" });
  }, [state, meeting, navigate]);

  return (
    <div className="flex h-screen min-h-0 flex-col overflow-hidden bg-surface">
      <TitleBar />
      <SystemStates />
      <UpdateBanner onOpenAbout={() => void navigate({ to: "/settings/$section", params: { section: "about" } })} />
      <div className="grid min-h-0 flex-1" style={{ gridTemplateColumns: compact ? "56px minmax(0,1fr)" : "216px minmax(0,1fr)" }}>
        <Sidebar compact={compact} />
        <main className="relative min-h-0 min-w-0 overflow-hidden">
          <Outlet />
        </main>
      </div>
      <UpdateReady />
      <CommandPalette />
      <DetectionPrompt />
      <ProcessingWatcher />
      <LiveAnnouncer />
    </div>
  );
}
