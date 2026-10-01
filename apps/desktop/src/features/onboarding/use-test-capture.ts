// SPDX-License-Identifier: Apache-2.0
// The 10 s test recording of D1 step 6. `testCapture` records briefly, deletes
// the meeting afterwards and streams its levels and first line through the
// core events, so this reads them from the live store like the live screen.
import { useCallback, useEffect, useRef, useState } from "react";
import { ipc } from "../../ipc";
import { isActive, useLive } from "../../state/live";

export type TestPhase = "idle" | "running" | "done" | "failed";
export const TEST_SECONDS = 10;
/** The core ends the run itself; this only guards against it never saying so. */
const GRACE_MS = 4000;

export type TestCapture = {
  phase: TestPhase;
  secondsLeft: number;
  /** dBFS, or null when the source is missing. */
  levels: { mic: number | null; system: number | null };
  /** The first transcript line (or its words so far), once there is one. */
  line: string | null;
  /** The system track stayed silent (system audio is off or nothing played). */
  systemSilent: boolean;
  /** Why it failed (a text from the core), when it did. */
  error: string | null;
  start: () => void;
};

export function useTestCapture(seconds = TEST_SECONDS): TestCapture {
  const [meeting, setMeeting] = useState<string | null>(null);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [now, setNow] = useState(0);
  const [failure, setFailure] = useState<string | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const liveMeeting = useLive((s) => s.meeting);
  const liveState = useLive((s) => s.state);
  const liveLevels = useLive((s) => s.levels);
  const lastLine = useLive((s) => s.lines.at(-1)?.text);
  const partial = useLive((s) => s.partial[0] ?? s.partial[1]);
  const silent = useLive((s) => s.capture.systemSilent);
  const lastError = useLive((s) => s.errors.at(-1)?.message);

  // Leaving the step: stop the clock and clear the test meeting from the live view.
  useEffect(
    () => () => {
      window.clearInterval(timer.current);
      if (!isActive(useLive.getState().state)) useLive.getState().reset();
    },
    [],
  );

  const start = useCallback(() => {
    window.clearInterval(timer.current);
    setFailure(null);
    setMeeting(null);
    const t0 = Date.now();
    setStartedAt(t0);
    setNow(t0);
    timer.current = window.setInterval(() => setNow(Date.now()), 200);
    void ipc.commands.testCapture(seconds).then((r) => {
      if (r.status === "ok") setMeeting(r.data);
      else {
        window.clearInterval(timer.current);
        setStartedAt(null);
        setFailure(r.error);
      }
    });
  }, [seconds]);

  const elapsed = startedAt == null ? 0 : now - startedAt;
  const mine = meeting != null && liveMeeting === meeting;
  const coreEnded = mine && (liveState === "ready" || liveState === "idle" || liveState === "failed");
  const timedOut = elapsed >= seconds * 1000 + GRACE_MS;
  const phase: TestPhase = failure != null || (mine && liveState === "failed") ? "failed" : startedAt == null ? "idle" : coreEnded || timedOut ? "done" : "running";
  // Stop the clock once it no longer matters.
  useEffect(() => {
    if (phase !== "running") window.clearInterval(timer.current);
  }, [phase]);

  const text = mine ? (lastLine ?? partial) : undefined;
  return {
    phase,
    secondsLeft: Math.max(0, Math.ceil(seconds - elapsed / 1000)),
    levels: mine ? liveLevels : { mic: null, system: null },
    line: text || null,
    systemSilent: mine && silent,
    error: failure ?? (phase === "failed" ? (lastError ?? "") : null),
    start,
  };
}
