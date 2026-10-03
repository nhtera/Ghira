// SPDX-License-Identifier: Apache-2.0
// Above every route: iOS shell events that apply everywhere. Dynamic Type
// drives the root font size (`--ghi-text-scale`); the other events (phase,
// interruption, ...) get their stores with 16-H.
import { Outlet } from "@tanstack/react-router";
import { useEffect } from "react";
import { ipc } from "../ipc";

export function RootView() {
  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    // f32 arrives as number | null (null: not finite).
    const setScale = (scale: number | null) =>
      document.documentElement.style.setProperty("--ghi-text-scale", String(Math.min(scale ?? 1, 2)));
    void (async () => {
      const u = await ipc.onMobileEvent((e) => {
        if (e.type === "textScale") setScale(e.scale);
      });
      if (!alive) return u();
      off = u;
      const s = await ipc.commands.lifecycleState().catch(() => null);
      if (alive && s?.status === "ok") setScale(s.data.textScale);
    })();
    return () => {
      alive = false;
      off?.();
    };
  }, []);
  return <Outlet />;
}
