// SPDX-License-Identifier: Apache-2.0
// The UI's only way to the Rust core: the tauri-specta commands and events
// (RT-14: the mock uses the same generated types, so screens built against
// it work unchanged on the real core).
import type { CoreEvent, MenuAction, commands } from "../bindings";

export type Commands = typeof commands;
export type Unlisten = () => void;

export interface Ipc {
  /** "tauri" in the app; "mock" in a plain browser (dev, tests, gallery). */
  readonly kind: "tauri" | "mock";
  readonly commands: Commands;
  onCoreEvent(cb: (e: CoreEvent) => void): Promise<Unlisten>;
  onMenuAction(cb: (a: MenuAction) => void): Promise<Unlisten>;
}

export const inTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
