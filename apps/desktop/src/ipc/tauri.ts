// SPDX-License-Identifier: Apache-2.0
import { commands, events } from "../bindings";
import type { Ipc } from "./ipc";

export const tauriIpc: Ipc = {
  kind: "tauri",
  commands,
  onCoreEvent: (cb) => events.coreEvent.listen((e) => cb(e.payload)),
  onMenuAction: (cb) => events.menuAction.listen((e) => cb(e.payload)),
};
