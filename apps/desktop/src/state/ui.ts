// SPDX-License-Identifier: Apache-2.0
// Window-level UI state: the command palette and the pending recording mode.
import { create } from "zustand";
import type { RecordMode } from "../bindings";

type Ui = {
  paletteOpen: boolean;
  setPaletteOpen: (open: boolean) => void;
  /** The mode the record control starts next (split button). */
  recordMode: RecordMode;
  setRecordMode: (m: RecordMode) => void;
};

export const useUi = create<Ui>((set) => ({
  paletteOpen: false,
  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
  recordMode: "call",
  setRecordMode: (recordMode) => set({ recordMode }),
}));
