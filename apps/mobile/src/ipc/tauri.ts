// SPDX-License-Identifier: Apache-2.0
import { convertFileSrc } from "@tauri-apps/api/core";
import { commands, events } from "../bindings";
import type { Ipc } from "./ipc";

export const tauriIpc: Ipc = {
  kind: "tauri",
  commands,
  onCoreEvent: (cb) => events.coreEvent.listen((e) => cb(e.payload)),
  onMobileEvent: (cb) => events.mobileEvent.listen((e) => cb(e.payload)),
  audioUrl: (token) => convertFileSrc(token, "ghi-audio"),
};
