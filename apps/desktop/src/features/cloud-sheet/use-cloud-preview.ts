// SPDX-License-Identifier: Apache-2.0
// The cloud sheet's state: what can be sent (keys, models, defaults) and the
// current preview. Every provider/model/redact/task change re-previews after a
// short debounce; Send always uses the id of the latest preview.
import { useEffect, useState } from "react";
import type {
  AppSettings,
  AskAnswer,
  CloudModel,
  CloudPreview,
  CloudTask,
} from "../../bindings";
import { ipc } from "../../ipc";

export const PREVIEW_DEBOUNCE_MS = 250;

export type PreviewState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; preview: CloudPreview }
  | { status: "answer"; answer: AskAnswer }
  | { status: "error"; message: string };

export type CloudChoices = {
  loaded: boolean;
  stored: Set<string>;
  providers: string[];
  models: CloudModel[];
  settings: AppSettings | null;
};

/** Keys, models and the saved defaults; read each time the sheet opens. */
export function useCloudChoices(open: boolean): CloudChoices {
  const [c, setC] = useState<CloudChoices>({
    loaded: false,
    stored: new Set(),
    providers: [],
    models: [],
    settings: null,
  });
  useEffect(() => {
    if (!open) return;
    let live = true;
    void Promise.all([
      ipc.commands.cloudKeys(),
      ipc.commands.cloudModels(),
      ipc.commands.getSettings(),
    ]).then(([keys, models, settings]) => {
      if (!live) return;
      const list = keys.status === "ok" ? keys.data : [];
      setC({
        loaded: true,
        stored: new Set(list.filter((k) => k.stored).map((k) => k.provider)),
        providers: list.map((k) => k.provider),
        models,
        settings: settings.status === "ok" ? settings.data : null,
      });
    });
    return () => {
      live = false;
    };
  }, [open]);
  return c;
}

export type Selection = { provider: string; model: string; redact: boolean };

/** The initial pick: the saved provider if it has a key, else the first with one. */
export function initialSelection(c: CloudChoices): Selection {
  const s = c.settings;
  const provider =
    s && c.stored.has(s.cloudProvider)
      ? s.cloudProvider
      : (c.providers.find((p) => c.stored.has(p)) ??
        s?.cloudProvider ??
        c.providers[0] ??
        "");
  const models = c.models.filter((m) => m.provider === provider);
  const model =
    s && models.some((m) => m.model === s.cloudModel)
      ? s.cloudModel
      : (models[0]?.model ?? "");
  return { provider, model, redact: s?.cloudRedact ?? true };
}

export function useCloudPreview(args: {
  meeting: string;
  task: CloudTask;
  sel: Selection | null;
  enabled: boolean;
}): { state: PreviewState; refresh: () => void } {
  const { meeting, task, sel, enabled } = args;
  const [done, setDone] = useState<{ key: string; state: PreviewState } | null>(
    null,
  );
  const [nonce, setNonce] = useState(0);
  const active = enabled && !!sel && !!sel.provider && !!sel.model;
  const key = JSON.stringify([
    meeting,
    task,
    sel?.provider,
    sel?.model,
    sel?.redact,
    nonce,
  ]);

  useEffect(() => {
    if (!active || !sel) return;
    const timer = window.setTimeout(async () => {
      const r = await ipc.commands.cloudPreview(meeting, {
        provider: sel.provider,
        model: sel.model,
        task,
        redact: sel.redact,
        extraNames: [],
      });
      let state: PreviewState;
      if (r.status === "error") state = { status: "error", message: r.error };
      else if (r.data.kind === "answer")
        state = { status: "answer", answer: r.data };
      else state = { status: "ready", preview: r.data };
      setDone({ key, state });
    }, PREVIEW_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
    // `key` captures every input; a stale response is dropped by the key check below.
  }, [key]); // eslint-disable-line react-hooks/exhaustive-deps

  // The old preview must not stay sendable while the next one is prepared.
  const state: PreviewState = !active
    ? { status: "idle" }
    : done?.key === key
      ? done.state
      : { status: "loading" };
  return { state, refresh: () => setNonce((n) => n + 1) };
}
