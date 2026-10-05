// SPDX-License-Identifier: Apache-2.0
// The M2 screen's connection to the core: the model (snapshot + events), the
// clock, the defaults a start needs, and the session commands.
import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import type {
  MicPermission,
  ProcessingTarget,
  MeetingLanguage,
} from "../../bindings";
import { ipc } from "../../ipc";
import { initialModel, isCapturing, reducer, type Action, type RecordModel } from "./model";

/** A snapshot slower than this is given up on (the core waits up to 10 s itself), so events never stay held. */
const SNAPSHOT_WAIT_MS = 4000;
/** After a failed snapshot, ask again this many times, this far apart. */
const SNAPSHOT_RETRIES = 3;
const SNAPSHOT_RETRY_MS = 1000;

export type StartAck = { consent: boolean; call: boolean; sensitive: boolean };
/** What the screen can say went wrong: a refused start, or a command that failed. */
export type RecordError =
  | "micInUse"
  | "diskLow"
  | "callActive"
  | "waitingForTranscription"
  | "pairingNotAvailable"
  | "sensitiveNeedsTranscript"
  | "generic"
  | "action"
  | "microphoneDenied"
  | "alreadyRecording";

/** What the shell reports once, not per session. */
export type RecordSetup = {
  callActive: boolean;
  mic: MicPermission;
  /** The device cannot transcribe live (Phone target off). */
  recordOnlyDevice: boolean;
  target: ProcessingTarget;
  language: MeetingLanguage;
};

const SETUP: RecordSetup = {
  callActive: false,
  mic: "notDetermined",
  recordOnlyDevice: false,
  target: "phone",
  language: "auto",
};

/** The errors that get a banner (the others change the screen instead). */
export type BannerError = Exclude<RecordError, "callActive" | "microphoneDenied" | "alreadyRecording">;
export const bannerError = (e: RecordError | null): BannerError | null => (e === null || e === "callActive" || e === "microphoneDenied" || e === "alreadyRecording" ? null : e);

/** The core's refusal codes (also tolerates the plain-words forms). */
export function startErrorFor(message: string): RecordError {
  const m = message.toLowerCase().replace(/[\s_-]/g, "");
  if (m.includes("microphonedenied")) return "microphoneDenied";
  if (m.includes("alreadyrecording")) return "alreadyRecording";
  if (m.includes("disklow")) return "diskLow";
  if (m.includes("callactive")) return "callActive";
  if (m.includes("micinuse") || m.includes("microphoneisinuse")) return "micInUse";
  if (m.includes("waitingfortranscription")) return "waitingForTranscription";
  if (m.includes("pairingnotavailable")) return "pairingNotAvailable";
  if (m.includes("sensitiveneedstranscript")) return "sensitiveNeedsTranscript";
  return "generic";
}

export function useRecordSetup() {
  const [setup, setSetup] = useState<RecordSetup>(SETUP);
  const patch = useCallback(
    (p: Partial<RecordSetup>) => setSetup((s) => ({ ...s, ...p })),
    [],
  );
  useEffect(() => {
    let alive = true;
    let off: (() => void) | undefined;
    const set = (p: Partial<RecordSetup>) => alive && patch(p);
    void (async () => {
      const u = await ipc.onMobileEvent((e) => {
        if (e.type === "callActive") set({ callActive: e.active });
      });
      if (!alive) return u();
      off = u;
      const [call, mic, tier, mobile, app] = await Promise.all([
        ipc.commands.recordCallActive(),
        ipc.commands.micPermission().catch(() => "notDetermined" as const),
        ipc.commands.deviceTier(),
        ipc.commands.mobileSettings(),
        ipc.commands.getSettings(),
      ]);
      set({
        ...(call.status === "ok" && { callActive: call.data }),
        mic,
        ...(tier.status === "ok" && {
          recordOnlyDevice: tier.data.tier === "recordOnly",
        }),
        ...(mobile.status === "ok" && { target: mobile.data.defaultTarget }),
        ...(app.status === "ok" && { language: app.data.meetingLanguage }),
      });
    })();
    // The microphone can be turned on in Settings while we are in the background.
    const recheck = () => {
      if (document.visibilityState === "visible")
        void ipc.commands.micPermission().then(
          (mic) => set({ mic }),
          () => {},
        );
    };
    document.addEventListener("visibilitychange", recheck);
    return () => {
      alive = false;
      off?.();
      document.removeEventListener("visibilitychange", recheck);
    };
  }, [patch]);
  return { setup, patch };
}

export function useRecord(setup: RecordSetup) {
  const [model, dispatch] = useReducer(reducer, initialModel);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<RecordError | null>(null);
  // While a snapshot is being read, events are held and replayed on top of it.
  const held = useRef<Action[] | null>(null);
  const reading = useRef(false);
  const readAgain = useRef(false);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const send = useCallback((a: Action) => {
    if (held.current) held.current.push(a);
    else dispatch(a);
  }, []);

  /** The snapshot, or null when the read failed or took too long (the events must go on either way). */
  const readSnapshot = useCallback(async () => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      const r = await Promise.race([
        ipc.commands.recordSnapshot(),
        new Promise<null>((resolve) => {
          timer = setTimeout(() => resolve(null), SNAPSHOT_WAIT_MS);
        }),
      ]);
      return r !== null && r.status === "ok" ? r.data : null;
    } catch {
      // The webview's bridge can fail right after a resume ("Load failed").
      return null;
    } finally {
      clearTimeout(timer);
    }
  }, []);

  const refresh = useCallback(async () => {
    // One read at a time: a second wake while one is open asks for another after it.
    if (reading.current) {
      readAgain.current = true;
      return;
    }
    reading.current = true;
    try {
      let retries = 0;
      for (;;) {
        readAgain.current = false;
        held.current = [];
        const state = await readSnapshot();
        const events = held.current ?? [];
        held.current = null;
        if (state) dispatch({ type: "snapshot", state });
        // Core events at or before the snapshot's seq are dropped by the reducer;
        // of the phases only the last is replayed (it may be newer than the snapshot).
        let lastPhase = -1;
        events.forEach((a, i) => {
          if (a.type === "mobile" && a.event.type === "phase") lastPhase = i;
        });
        events.forEach((a, i) => {
          if (state && a.type === "mobile" && a.event.type === "phase" && i !== lastPhase) return;
          dispatch(a);
        });
        if (readAgain.current) continue;
        // Without a snapshot the lines of the gap are missing: try again shortly (the events flow meanwhile).
        if (state || retries >= SNAPSHOT_RETRIES || !alive.current) break;
        retries += 1;
        await new Promise((r) => setTimeout(r, SNAPSHOT_RETRY_MS));
        if (!alive.current) break;
      }
    } finally {
      held.current = null;
      reading.current = false;
    }
  }, [readSnapshot]);

  useEffect(() => {
    let alive = true;
    const offs: (() => void)[] = [];
    void (async () => {
      const [core, mobile] = await Promise.all([ipc.onCoreEvent((env) => send({ type: "core", env })), ipc.onMobileEvent((event) => send({ type: "mobile", event }))]);
      if (!alive) {
        core();
        mobile();
        return;
      }
      offs.push(core, mobile);
      await refresh();
    })();
    // The webview was asleep (locked): the clock and the transcript need the truth.
    const wake = () => document.visibilityState === "visible" && void refresh();
    document.addEventListener("visibilitychange", wake);
    return () => {
      alive = false;
      offs.forEach((off) => off());
      document.removeEventListener("visibilitychange", wake);
    };
  }, [refresh, send]);

  // The clock counts recorded time (a pause does not count).
  const running = isCapturing(model);
  useEffect(() => {
    if (!running) return;
    const id = setInterval(() => dispatch({ type: "tick" }), 1000);
    return () => clearInterval(id);
  }, [running]);

  // An interruption is answered by the user; ask the core what it was.
  useEffect(() => {
    if (model.phase !== "interrupted") return;
    void ipc.commands.recordResumePrompt().then((r) => r.status === "ok" && dispatch({ type: "prompt", prompt: r.data }));
  }, [model.phase]);

  // The phase event does not say why there is no transcript; the snapshot does.
  const { phase, recordOnlyReason } = model;
  useEffect(() => {
    if (phase !== "recordOnly" || recordOnlyReason !== null) return;
    void ipc.commands.recordSnapshot().then((r) => r.status === "ok" && dispatch({ type: "reason", reason: r.data.recordOnlyReason }));
  }, [phase, recordOnlyReason]);

  const start = useCallback(
    async (ack: StartAck) => {
      setStarting(true);
      setError(null);
      dispatch({ type: "dismissSaved" });
      const r = await ipc.commands.recordStart({ mode: "room", language: setup.language, title: null, target: setup.target, consentAcknowledged: ack.consent, callAcknowledged: ack.call, sensitive: ack.sensitive });
      setStarting(false);
      if (r.status === "error") {
        const kind = startErrorFor(r.error);
        setError(kind);
        return kind;
      }
      return null;
    },
    [setup.language, setup.target],
  );

  const run = useCallback(async (command: () => Promise<{ status: "ok" | "error" }>) => {
    setError(null);
    const r = await command().catch(() => ({ status: "error" as const }));
    if (r.status === "error") setError("action");
  }, []);

  return {
    model: model satisfies RecordModel,
    starting,
    error,
    dismissError: () => setError(null),
    dismissSaved: () => dispatch({ type: "dismissSaved" }),
    start,
    pause: () => void run(ipc.commands.recordPause),
    resume: () => void run(ipc.commands.recordResume),
    stop: () => void run(ipc.commands.recordStop),
    mark: () => void run(ipc.commands.recordMark),
    /** Sensitive mode on for the running recording (one way). Resolves whether it worked. */
    makeSensitive: async () => {
      const r = await ipc.commands.recordSetSensitive(true).catch(() => null);
      if (r?.status !== "ok") setError("action");
      return r?.status === "ok";
    },
    discardPreview: (seconds: number) => ipc.commands.recordDiscardPreview(seconds),
    /** Discards from the previewed cut on. Resolves whether it worked. */
    discardFrom: async (fromMs: number) => {
      const r = await ipc.commands.recordDiscardFrom(fromMs).catch(() => null);
      if (r?.status !== "ok") setError("action");
      return r?.status === "ok";
    },
  };
}
