// SPDX-License-Identifier: Apache-2.0
// iOS spike screen (phase 7): record, live transcript with speakers, spike
// metrics, and the 1,500-line scroll test. Not the v1 mobile UI (phase 16).
import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type Line, type Phase, type Status } from "./bindings";
import { ScrollTest } from "./scroll-test";
import { TranscriptList } from "./transcript-list";

const POLL_MS = 500;
const inTauri = "__TAURI_INTERNALS__" in window;

const PHASE_LABEL: Record<Phase, string> = {
  loading: "Loading models",
  live: "Live",
  locked: "Locked: recording only",
  catchingUp: "Catching up",
  hot: "Too hot: recording only",
  interrupted: "Interrupted",
  finishing: "Finishing transcript",
  done: "Done",
  recordOnly: "Recording only",
};

const THERMAL = ["nominal", "fair", "serious", "critical"];

function clock(s: number): string {
  const t = Math.floor(s);
  const mm = String(Math.floor(t / 60) % 60).padStart(2, "0");
  const ss = String(t % 60).padStart(2, "0");
  return t >= 3600 ? `${Math.floor(t / 3600)}:${mm}:${ss}` : `${mm}:${ss}`;
}

function num(v: number | null | undefined, digits = 2, suffix = ""): string {
  return v == null ? "–" : `${v.toFixed(digits)}${suffix}`;
}

export function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [lines, setLines] = useState<Line[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [showTest, setShowTest] = useState(false);
  const known = useRef<{ id: string | null; count: number }>({ id: null, count: 0 });

  const inFlight = useRef(false);

  const poll = useCallback(async () => {
    // The interval, visibilitychange and actions can fire together; one poll
    // at a time, or the same lines get appended twice.
    if (!inTauri || inFlight.current) return;
    inFlight.current = true;
    try {
      const k = known.current;
      const s = await commands.snapshot(k.count);
      const snap = s.session;
      if (snap) {
        if (snap.id !== k.id) {
          // A new recording: start over.
          k.id = snap.id;
          k.count = 0;
          setLines([]);
          if (snap.lineCount > 0) {
            const full = await commands.snapshot(0);
            setLines(full.session?.lines ?? []);
            k.count = full.session?.lineCount ?? 0;
          }
        } else if (snap.lines.length > 0) {
          setLines((prev) => prev.concat(snap.lines));
          k.count = snap.lineCount;
        }
      }
      setStatus(s);
    } finally {
      inFlight.current = false;
    }
  }, []);

  useEffect(() => {
    // The webview is suspended while locked; a poll on return fetches every
    // line produced meanwhile (the transcript lives in Rust).
    const id = window.setInterval(() => void poll().catch(() => {}), POLL_MS);
    const onVisible = () => void poll().catch(() => {});
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [poll]);

  const snap = status?.session ?? null;
  const recording = snap?.recording ?? false;
  const missing = status?.models.filter((m) => !m.ready) ?? [];

  async function start() {
    setError(null);
    const r = await commands.startRecording();
    if (r.status === "error") setError(r.error);
    await poll();
  }

  if (showTest) return <ScrollTest onClose={() => setShowTest(false)} />;

  return (
    <main className="screen">
      <header className="bar">
        <div>
          <h1>Ghira Spike</h1>
          <span className={`chip phase-${snap?.phase ?? "idle"}`}>{snap ? PHASE_LABEL[snap.phase] : "Ready"}</span>
        </div>
        <div className="timer" aria-label="Elapsed">
          {clock(snap?.elapsedS ?? 0)}
        </div>
      </header>

      <div className="actions">
        {recording ? (
          <>
            <button className="stop" onClick={() => void commands.stopRecording().then(poll)}>
              Stop
            </button>
            <button onClick={() => void commands.markMoment().then(poll)}>Mark ({snap?.marks.length ?? 0})</button>
          </>
        ) : (
          <button className="record" onClick={() => void start()}>
            Record
          </button>
        )}
        <button className="ghost" onClick={() => setShowTest(true)}>
          Scroll test
        </button>
      </div>

      {(error ?? snap?.error) && <p className="notice">{error ?? snap?.error}</p>}
      {status && !status.engineBuilt && <p className="notice">This build has no speech engine: it records only.</p>}
      {status?.engineBuilt && missing.length > 0 && (
        <p className="notice">
          Models missing ({missing.map((m) => m.id).join(", ")}): run apps/mobile/scripts/push-models.sh. Recording
          still works.
        </p>
      )}

      <TranscriptList lines={lines} partial={snap?.partial ?? ""} />

      {snap && (
        <dl className="stats">
          <dt>Recorded</dt>
          <dd>{clock(snap.stats.recordedS ?? 0)}</dd>
          <dt>Backlog</dt>
          <dd>{num(snap.stats.backlogS, 1, " s")}</dd>
          <dt>RTF</dt>
          <dd>{num(snap.stats.rtf)}</dd>
          <dt>Catch-up</dt>
          <dd>{num(snap.stats.catchUpX, 1, "×")}</dd>
          <dt>Model load</dt>
          <dd>{num(snap.stats.modelLoadS, 1, " s")}</dd>
          <dt>Thermal</dt>
          <dd>{snap.stats.thermal == null ? "–" : (THERMAL[snap.stats.thermal] ?? snap.stats.thermal)}</dd>
          <dt>Memory</dt>
          <dd>{num(snap.stats.memoryMb, 0, " MB")}</dd>
          <dt>Battery</dt>
          <dd>{snap.stats.battery == null ? "–" : `${Math.round(snap.stats.battery * 100)}%`}</dd>
          <dt>GPU overlaps</dt>
          <dd>
            {snap.stats.gpuOverlaps} / {snap.stats.engineResets} resets
          </dd>
          <dt>Dropped</dt>
          <dd>{num((snap.stats.droppedSamples ?? 0) / 16000, 2, " s")}</dd>
        </dl>
      )}
    </main>
  );
}
