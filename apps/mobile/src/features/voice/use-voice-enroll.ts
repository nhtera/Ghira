// SPDX-License-Identifier: Apache-2.0
// "Me" enrollment on the phone, as one small state machine: start the mic, poll
// the level while the passage is read, finish (automatically when the buffer is
// full) or cancel. The mic is released when the component goes away. Consent is
// the caller's job: it is given before `start`.
import { useCallback, useEffect, useRef, useState } from "react";
import { ipc } from "../../ipc";

export type EnrollPhase = "idle" | "starting" | "reading" | "saving" | "done";

export type EnrollState = {
  phase: EnrollPhase;
  /** 0..1, the latest moment's loudness. */
  level: number | null;
  seconds: number;
  maxSeconds: number;
  /** The core's error code from the last start/finish, if it failed. */
  error: string | null;
};

/** The core needs 10 s of speech; the clock counts pauses too, so this leaves room for them. */
export const MIN_SECONDS = 15;
const POLL_MS = 150;
const INITIAL: EnrollState = {
  phase: "idle",
  level: null,
  seconds: 0,
  maxSeconds: 25,
  error: null,
};

export function useVoiceEnroll(onSaved?: () => void) {
  const [state, setState] = useState<EnrollState>(INITIAL);
  // The mic is open between a successful start and finish/cancel.
  const open = useRef(false);
  const finishing = useRef(false);
  // Set by leaving or cancelling: a start still in flight must not leave the mic open.
  const cancelled = useRef(false);
  const saved = useRef(onSaved);
  useEffect(() => {
    saved.current = onSaved;
  });

  const cancel = useCallback(() => {
    cancelled.current = true;
    if (open.current) {
      open.current = false;
      void ipc.commands.voiceEnrollCancel();
    }
    setState(INITIAL);
  }, []);
  useEffect(() => {
    cancelled.current = false;
    return () => {
      cancelled.current = true;
      if (open.current) void ipc.commands.voiceEnrollCancel();
    };
  }, []);

  const finish = useCallback(async () => {
    if (!open.current || finishing.current) return;
    finishing.current = true;
    // The core owns the buffer from here: leaving while it saves must not cancel.
    open.current = false;
    setState((s) => ({ ...s, phase: "saving" }));
    const r = await ipc.commands.voiceEnrollStop();
    finishing.current = false;
    // Saved even if the user already left: whoever shows the profile reloads.
    if (r.status === "ok") saved.current?.();
    if (cancelled.current) return;
    if (r.status === "error") setState({ ...INITIAL, error: r.error });
    else setState((s) => ({ ...s, phase: "done", error: null }));
  }, []);

  const start = useCallback(async () => {
    cancelled.current = false;
    setState({ ...INITIAL, phase: "starting" });
    const r = await ipc.commands.voiceEnrollStart();
    if (r.status === "error") {
      if (!cancelled.current) setState({ ...INITIAL, error: r.error });
      return;
    }
    if (cancelled.current) {
      // Left while the mic was opening: close it again.
      void ipc.commands.voiceEnrollCancel();
      return;
    }
    open.current = true;
    setState((s) => ({ ...s, phase: "reading" }));
  }, []);

  const reading = state.phase === "reading";
  useEffect(() => {
    if (!reading) return;
    let gone = false;
    const timer = window.setInterval(() => {
      void ipc.commands.enrollVoiceLevel().then((r) => {
        if (gone) return;
        // The core dropped the enrollment (locked, a recording started): start again.
        if (r.status === "error") {
          open.current = false;
          setState({ ...INITIAL, error: r.error });
          return;
        }
        const l = r.data;
        setState((s) =>
          s.phase === "reading"
            ? {
                ...s,
                level: l.level,
                seconds: l.seconds ?? s.seconds,
                maxSeconds: l.maxSeconds ?? s.maxSeconds,
              }
            : s,
        );
        // The buffer is full or the mic closed: save what was read.
        if (l.done) void finish();
      });
    }, POLL_MS);
    return () => {
      gone = true;
      window.clearInterval(timer);
    };
  }, [reading, finish]);

  const canFinish = reading && state.seconds >= MIN_SECONDS;
  return { state, canFinish, start, finish, cancel };
}
