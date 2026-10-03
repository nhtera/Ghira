// SPDX-License-Identifier: Apache-2.0
// The UI's only way to the Rust core: the tauri-specta commands and events
// (RT-14: the mock uses the same generated types, so screens built against
// it work unchanged on the real core).
import type { CoreEvent, MobileEvent, commands } from "../bindings";

export type Commands = typeof commands;
export type Unlisten = () => void;

export interface Ipc {
  /** "tauri" in the app; "mock" in a plain browser (dev, tests). */
  readonly kind: "tauri" | "mock";
  readonly commands: Commands;
  /** Transcript, speakers, jobs: the events shared with the desktop. */
  onCoreEvent(cb: (e: CoreEvent) => void): Promise<Unlisten>;
  /** What the iOS shell tells the webview (phase, backlog, pocket, interruption, thermal, text scale, inbox). */
  onMobileEvent(cb: (e: MobileEvent) => void): Promise<Unlisten>;
  /** The URL an `<audio>` element plays for a ghi-audio token (issueAudioPlay). */
  audioUrl(token: string): string;
}

export const inTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
