// SPDX-License-Identifier: Apache-2.0
// Staged files and the import queue outside React, so a drop that arrives
// before /import mounts (the core navigates there) and updates that arrive
// while another screen is open are not lost. `useImportListeners` feeds it;
// it is mounted once at the app root.
import { useEffect } from "react";
import { create } from "zustand";
import type { StagedFile } from "../../bindings";
import { ipc } from "../../ipc";
import { queueReducer, type Queue, type QueueAction } from "./import-model";

type State = {
  staged: StagedFile[];
  queue: Queue;
  addStaged: (files: StagedFile[]) => void;
  removeStaged: (ids: string[]) => void;
  dispatch: (a: QueueAction) => void;
};

export const useImportStore = create<State>((set) => ({
  staged: [],
  queue: {},
  addStaged: (files) =>
    set((s) => {
      const known = new Set(s.staged.map((f) => f.id));
      const fresh = files.filter((f) => !known.has(f.id));
      return fresh.length ? { staged: [...s.staged, ...fresh] } : s;
    }),
  removeStaged: (ids) => set((s) => ({ staged: s.staged.filter((f) => !ids.includes(f.id)) })),
  dispatch: (a) => set((s) => ({ queue: queueReducer(s.queue, a) })),
}));

/** Subscribes to dropped files and queue updates (safe to mount twice: staging dedupes by id, queue updates are idempotent). */
export function useImportListeners() {
  useEffect(() => {
    let gone = false;
    const offs: (() => void)[] = [];
    const keep = (p: Promise<() => void>) => void p.then((u) => (gone ? u() : offs.push(u)));
    // The core keeps dropped files until read (a cold launch from the Dock fires the event before the page listens): read on mount and on each event.
    const take = () => void ipc.commands.takeDroppedFiles().then((files) => useImportStore.getState().addStaged(files));
    take();
    keep(ipc.onImportStaged(take));
    keep(ipc.onImportUpdate((update) => useImportStore.getState().dispatch({ type: "update", update })));
    return () => {
      gone = true;
      offs.forEach((u) => u());
    };
  }, []);
}
