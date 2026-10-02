// SPDX-License-Identifier: Apache-2.0
// The UI's only way to the Rust core: the tauri-specta commands and events
// (RT-14: the mock uses the same generated types, so screens built against
// it work unchanged on the real core).
import type {
  CoreEvent,
  ImportStaged,
  ImportUpdate,
  LockChanged,
  MeetingDetected,
  UpdateChanged,
  MenuAction,
  ModelDownload,
  Navigate,
  QuitRequested,
  commands,
} from "../bindings";

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
  /** Files dropped on the window or the Dock were staged (read them with stagedFiles). */
  onImportStaged(cb: (e: ImportStaged) => void): Promise<Unlisten>;
  /** A queued import moved on (queued, decoding with progress, done, failed, cancelled). */
  onImportUpdate(cb: (e: ImportUpdate) => void): Promise<Unlisten>;
  /** The app lock engaged or was lifted (every window). */
  onLockChanged(cb: (e: LockChanged) => void): Promise<Unlisten>;
  /** The app-update status changed (a check, a download, an error). */
  onUpdateChanged(cb: (e: UpdateChanged) => void): Promise<Unlisten>;
  /** The URL an `<audio>` element plays for a ghi-audio token (issueAudioPlay, issueAudioSample). */
  audioUrl(token: string): string;
}

export const inTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
