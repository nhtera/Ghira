// SPDX-License-Identifier: Apache-2.0
// A scripted core for the browser: the same command and event types as the
// real one (bindings.ts). Commands nobody scripted yet answer like the real
// stubs ("not mocked"), so a screen sees its error path, never a crash.
// Tests drive events through `window.__ghiMock`.
import type { CoreEvent, MobileEvent } from "../bindings";
import type { Commands, Ipc } from "./ipc";
import { calendarCommands } from "./mock-calendar";
import { meetingCommands } from "./mock-meetings";
import { recordCommands } from "./mock-record";
import { createSyncMock } from "./mock-sync";
import { gateContent, lockListeners, settingsCommands } from "./mock-settings";

const coreListeners = new Set<(e: CoreEvent) => void>();
const mobileListeners = new Set<(e: MobileEvent) => void>();

const sync = createSyncMock();

const scripted: Partial<Commands> = {
  appVersion: async () => ({ app: "0.1.0", core: "0.1.0" }),
  micPermission: async () => "granted",
  openAppSettings: async () => undefined,
  ...recordCommands,
  ...meetingCommands,
  ...settingsCommands,
  ...calendarCommands,
  ...sync.commands,
};

/**
 * Commands that return their value directly (no `{ status, data }` result):
 * unscripted, they throw like a failed invoke instead of answering a result.
 * mock.test.ts checks this list against bindings.ts.
 */
export const RAW_COMMANDS = new Set(["appVersion", "cloudModels", "micPermission", "openAppSettings", "requestMicPermission"]);

/** Answers scripted commands; the rest fail like the real "not yet" stubs. */
export function scriptedCommands(script: Partial<Commands>): Commands {
  return new Proxy(script, {
    get(target, name: string) {
      const scriptedCommand = (target as Record<string, unknown>)[name];
      if (scriptedCommand) return scriptedCommand;
      if (RAW_COMMANDS.has(name)) {
        return async () => {
          throw new Error(`not mocked: ${name}`);
        };
      }
      return async () => ({ status: "error", error: `not mocked: ${name}` });
    },
  }) as Commands;
}

// Locked or starting (driven by mock-settings): content commands refuse like Rust.
const commands = scriptedCommands(gateContent(scripted));

export const mockIpc: Ipc = {
  kind: "mock",
  commands,
  onCoreEvent: async (cb) => {
    coreListeners.add(cb);
    return () => coreListeners.delete(cb);
  },
  onMobileEvent: async (cb) => {
    mobileListeners.add(cb);
    return () => mobileListeners.delete(cb);
  },
  onLockChanged: async (cb) => {
    lockListeners.add(cb);
    return () => lockListeners.delete(cb);
  },
  onSyncEvent: async (cb) => {
    sync.listeners.add(cb);
    return () => sync.listeners.delete(cb);
  },
  audioUrl: (token) => token,
};

declare global {
  interface Window {
    __ghiMock?: {
      simulateMobileEvent(e: MobileEvent): void;
      simulateCoreEvent(e: CoreEvent): void;
    } & ReturnType<typeof createSyncMock>["hooks"];
  }
}

window.__ghiMock = {
  simulateMobileEvent: (e) => mobileListeners.forEach((l) => l(e)),
  simulateCoreEvent: (e) => coreListeners.forEach((l) => l(e)),
  ...sync.hooks,
};
