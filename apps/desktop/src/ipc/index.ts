// SPDX-License-Identifier: Apache-2.0
// The IPC in use: the real core inside Tauri, the scripted mock elsewhere
// (vite dev in a browser, Playwright). The mock is its own chunk, never
// loaded in the app.
import { inTauri, type Ipc } from "./ipc";
import { tauriIpc } from "./tauri";

export const ipc: Ipc = inTauri() ? tauriIpc : (await import("./mock")).mockIpc;
export type { Ipc } from "./ipc";
