// SPDX-License-Identifier: Apache-2.0
import { convertFileSrc } from "@tauri-apps/api/core";
import { commands, events } from "../bindings";
import type { Ipc } from "./ipc";

export const tauriIpc: Ipc = {
  kind: "tauri",
  commands,
  onCoreEvent: (cb) => events.coreEvent.listen((e) => cb(e.payload)),
  onMenuAction: (cb) => events.menuAction.listen((e) => cb(e.payload)),
  onMeetingDetected: (cb) => events.meetingDetected.listen((e) => cb(e.payload)),
  onNavigate: (cb) => events.navigate.listen((e) => cb(e.payload)),
  onQuitRequested: (cb) => events.quitRequested.listen((e) => cb(e.payload)),
  onModelDownload: (cb) => events.modelDownload.listen((e) => cb(e.payload)),
  onImportStaged: (cb) => events.importStaged.listen((e) => cb(e.payload)),
  onImportUpdate: (cb) => events.importUpdate.listen((e) => cb(e.payload)),
  onUpdateChanged: (cb) => events.updateChanged.listen((e) => cb(e.payload)),
  onLockChanged: (cb) => events.lockChanged.listen((e) => cb(e.payload)),
  onSyncEvent: (cb) => events.syncEvent.listen((e) => cb(e.payload)),
  audioUrl: (token) => convertFileSrc(token, "ghi-audio"),
};
