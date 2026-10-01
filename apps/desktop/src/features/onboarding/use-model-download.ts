// SPDX-License-Identifier: Apache-2.0
// Model status and download progress for onboarding. Lives in the flow (not in
// the step) so a download keeps going, and keeps reporting, while the user
// moves on: "continue while downloading" never blocks (D1).
import { useCallback, useEffect, useMemo, useState } from "react";
import type { ModelDownload, ModelInfo, ModelsStatus } from "../../bindings";
import { ipc } from "../../ipc";

export type DownloadPhase = "loading" | "idle" | "downloading" | "failed" | "done";

export type ModelProgress = {
  model: ModelInfo;
  installed: boolean;
  /** The download of this model is running (or being verified). */
  active: boolean;
  failed: boolean;
  /** 0..100; includes an earlier partial download. */
  percent: number;
};

export type ModelDownloadState = {
  status: ModelsStatus | null;
  models: ModelProgress[];
  phase: DownloadPhase;
  /** 0..100 over all models. */
  percent: number;
  /** A rough estimate from the speed so far; null until there is one. */
  minutesLeft: number | null;
  error: string | null;
  totalBytes: number;
  start: () => void;
  /** Re-read the status (a core error may mean a model went bad). */
  refresh: () => void;
  cancel: () => void;
};

const pct = (done: number, total: number) => (total > 0 ? Math.min(100, Math.round((done / total) * 100)) : 0);

export function useModelDownload(): ModelDownloadState {
  const [status, setStatus] = useState<ModelsStatus | null>(null);
  const [events, setEvents] = useState<Record<string, ModelDownload>>({});
  const [error, setError] = useState<string | null>(null);
  // From the press of Download until it ends, fails or is cancelled (models
  // download one after another, so "none active" alone doesn't mean idle).
  const [running, setRunning] = useState(false);
  // The first progress seen per model: the baseline for the speed estimate.
  const [first, setFirst] = useState<Record<string, { at: number; done: number }>>({});
  const [now, setNow] = useState(() => Date.now());

  const refresh = useCallback(
    () =>
      void ipc.commands.modelsStatus().then((r) => {
        if (r.status === "ok") setStatus(r.data);
        else setError(r.error);
      }),
    [],
  );

  useEffect(() => {
    let alive = true;
    let off: (() => void) | undefined;
    refresh();
    void ipc
      .onModelDownload((e) => {
        if (!alive) return;
        setEvents((prev) => ({ ...prev, [e.model]: e }));
        setNow(Date.now());
        if (e.phase === "downloading") setFirst((p) => (p[e.model] ? p : { ...p, [e.model]: { at: Date.now(), done: e.done ?? 0 } }));
        if (e.phase === "failed") setError(e.error ?? "");
        if (e.phase === "failed" || e.phase === "cancelled") setRunning(false);
        if (e.phase === "done" || e.phase === "cancelled") refresh();
      })
      .then((u) => (alive ? (off = u) : u()));
    return () => {
      alive = false;
      off?.();
    };
  }, [refresh]);

  const models = useMemo<ModelProgress[]>(
    () =>
      (status?.models ?? []).map((m) => {
        const ev = events[m.id];
        // A file that failed its checksum is installed but not good: it counts again until re-downloaded.
        const installed = (m.installed && !m.damaged) || ev?.phase === "done";
        const total = m.size ?? ev?.total ?? 0;
        const got = installed ? total : Math.max(ev?.done ?? 0, m.partialBytes ?? 0);
        return {
          model: m,
          installed,
          active: ev?.phase === "downloading" || ev?.phase === "verifying",
          failed: ev?.phase === "failed",
          percent: installed ? 100 : pct(got, total),
        };
      }),
    [status, events],
  );

  const totalBytes = models.reduce((n, m) => n + (m.model.size ?? 0), 0);
  const gotBytes = models.reduce((n, m) => n + ((m.model.size ?? 0) * m.percent) / 100, 0);
  const percent = pct(gotBytes, totalBytes);
  const allDone = models.length > 0 && models.every((m) => m.installed);
  const anyActive = models.some((m) => m.active);
  const anyFailed = models.some((m) => m.failed);
  const phase: DownloadPhase = !status ? "loading" : allDone ? "done" : anyFailed || (error && !anyActive) ? "failed" : anyActive || running ? "downloading" : "idle";

  // Speed since the first progress: enough for "about N min left".
  const bases = Object.values(first);
  let minutesLeft: number | null = null;
  if (phase === "downloading" && bases.length > 0) {
    const since = Math.min(...bases.map((b) => b.at));
    const moved = Object.entries(first).reduce((n, [id, b]) => n + Math.max(0, (events[id]?.done ?? 0) - b.done), 0);
    if (now > since && moved > 0) minutesLeft = Math.max(1, Math.ceil((totalBytes - gotBytes) / (moved / (now - since)) / 60_000));
  }

  const start = useCallback(() => {
    setError(null);
    setRunning(true);
    setFirst({});
    setEvents((prev) => Object.fromEntries(Object.entries(prev).filter(([, e]) => e.phase !== "failed" && e.phase !== "cancelled")));
    void ipc.commands.downloadModels().then((r) => {
      if (r.status === "error") {
        setError(r.error);
        setRunning(false);
      }
      refresh();
    });
  }, [refresh]);

  const cancel = useCallback(() => {
    void Promise.resolve(ipc.commands.cancelModelDownload()).then(() => {
      setRunning(false);
      refresh();
    });
  }, [refresh]);

  return { status, models, phase, percent, minutesLeft, error, totalBytes, start, cancel, refresh };
}
