// SPDX-License-Identifier: Apache-2.0
// The UI's only way to the Rust core: the tauri-specta commands and events
// (RT-14: the mock uses the same generated types, so screens built against
// it work unchanged on the real core).
import type { CoreEvent, MeetingDetected, MenuAction, ModelDownload, Navigate, QuitRequested, commands } from "../bindings";

export type Commands = typeof commands;
export type Unlisten = () => void;

export interface Ipc {
  /** "tauri" in the app; "mock" in a plain browser (dev, tests, gallery). */
  readonly kind: "tauri" | "mock";
  readonly commands: Commands;
  onCoreEvent(cb: (e: CoreEvent) => void): Promise<Unlisten>;
  onMenuAction(cb: (a: MenuAction) => void): Promise<Unlisten>;
  /** A meeting app started using the mic (the UI shows the prompt). */
  onMeetingDetected(cb: (e: MeetingDetected) => void): Promise<Unlisten>;
  /** Rust brought a window forward for a route. */
  onNavigate(cb: (e: Navigate) => void): Promise<Unlisten>;
  /** Model download progress (onboarding, Settings → Models). */
  onModelDownload(cb: (e: ModelDownload) => void): Promise<Unlisten>;
  /** Quit while recording: ask "Stop and quit?" and answer with quitApp. */
  onQuitRequested(cb: (e: QuitRequested) => void): Promise<Unlisten>;
}

export const inTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
