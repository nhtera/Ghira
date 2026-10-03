// SPDX-License-Identifier: Apache-2.0
// The input level as bars, newest on the right. Decorative (the timer and the
// transcript carry the state). Reduced motion: the bars hold one still shape.
import { cn } from "@ghi/ui";
import { memo, useEffect, useState } from "react";
import { ipc } from "../../ipc";
import { LEVELS, pushLevel } from "./model";

const FLOOR_DB = -60;
/** The still shape for reduced motion and for a paused recording. */
const STILL = Array.from(
  { length: LEVELS },
  (_, i) => 0.18 + 0.12 * Math.abs(Math.sin(i * 0.9)),
);

function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(
    () =>
      typeof matchMedia === "function" &&
      matchMedia("(prefers-reduced-motion: reduce)").matches,
  );
  useEffect(() => {
    if (typeof matchMedia !== "function") return;
    const mq = matchMedia("(prefers-reduced-motion: reduce)");
    const on = () => setReduced(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  return reduced;
}

/** The mic level window, from its own subscription: ten updates a second must not re-render the screen. */
function useLevels(active: boolean): number[] {
  const [levels, setLevels] = useState<number[]>([]);
  useEffect(() => {
    if (!active) return;
    let alive = true;
    let off: (() => void) | undefined;
    void ipc
      .onCoreEvent((env) => {
        if (env.event.type === "levelMeter" && env.event.micDbfs !== null) {
          const db = env.event.micDbfs;
          setLevels((l) => pushLevel(l, db));
        }
      })
      .then((u) => (alive ? (off = u) : u()));
    return () => {
      alive = false;
      off?.();
      setLevels([]);
    };
  }, [active]);
  return levels;
}

export const Waveform = memo(function Waveform({ active, className }: { active: boolean; className?: string }) {
  const levels = useLevels(active);
  const reduced = useReducedMotion();
  const moving = active && !reduced;
  const heights = STILL.map((still, i) => {
    if (!moving) return still;
    const db = levels[levels.length - LEVELS + i];
    return db === undefined ? 0.06 : Math.min(1, Math.max(0.06, (Math.min(0, db) - FLOOR_DB) / -FLOOR_DB));
  });
  return (
    <div aria-hidden="true" data-testid="waveform" data-moving={moving ? "true" : "false"} className={cn("flex h-12 shrink-0 items-center justify-between gap-[3px]", className)}>
      {heights.map((h, i) => (
        <span key={i} className={cn("w-1 rounded-full", active ? "bg-accent" : "bg-line2", moving && "transition-[height] duration-100 ease-linear")} style={{ height: `${Math.round(h * 100)}%` }} />
      ))}
    </div>
  );
});
