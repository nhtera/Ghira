// SPDX-License-Identifier: Apache-2.0

// The live demo's clock: meeting seconds, ticking every 100 ms at 2.4x.
// Drawn at 27 s on the server and on first render. Stops while the demo is
// off-screen or paused; with reduced motion it never ticks and shows the
// finished meeting.

import { useEffect, useRef, useState } from "react";
import { DEMO_START, END, nextTime, STEP } from "@/content/demo-data";
import { REDUCED_MOTION, useMediaQuery } from "./use-media-query";

export function useDemoClock() {
  const reduced = useMediaQuery(REDUCED_MOTION);
  const [now, setNow] = useState(DEMO_START);
  const [paused, setPaused] = useState(false);
  const onScreen = useRef(true);
  const ref = useRef<HTMLDivElement>(null);
  const playing = !reduced && !paused;

  useEffect(() => {
    if (!playing) return;
    const id = setInterval(() => {
      if (onScreen.current) setNow(nextTime);
    }, STEP * 1000);
    return () => clearInterval(id);
  }, [playing]);

  useEffect(() => {
    const el = ref.current;
    if (!el || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver((entries) => {
      onScreen.current = entries[entries.length - 1].isIntersecting;
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  return { ref, time: reduced ? END : now, playing, reduced, toggle: () => setPaused((p) => !p) };
}
