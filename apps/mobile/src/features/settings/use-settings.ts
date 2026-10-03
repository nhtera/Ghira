// SPDX-License-Identifier: Apache-2.0
import { useCallback } from "react";
import type {
  AppSettings,
  MobileSettings,
  SettingsPatch,
} from "../../bindings";
import { ipc } from "../../ipc";
import { setLockEnabled } from "../app-lock";
import { unwrap, useAction, useResource } from "./api";

const loadApp = async () => unwrap(await ipc.commands.getSettings());
const loadMobile = async () => unwrap(await ipc.commands.mobileSettings());

/** The shared app settings (get/update_settings); `patch` saves and adopts what Rust returns. */
export function useAppSettings() {
  const r = useResource<AppSettings>(loadApp);
  const action = useAction();
  const { set } = r;
  const patch = useCallback(
    (p: SettingsPatch) =>
      action.run(async () => {
        set(unwrap(await ipc.commands.updateSettings(p)));
        return true;
      }),
    [action, set],
  );
  /** App lock on/off or its idle time; turning it on or off asks for Face ID first. */
  const setLock = useCallback(
    (on: boolean, afterMinutes: number, reason: string) =>
      action.run(async () => {
        const next = unwrap(
          await ipc.commands.setAppLock(on, afterMinutes, reason),
        );
        setLockEnabled(next.appLock);
        set(next);
        return true;
      }),
    [action, set],
  );
  return {
    settings: r.data,
    loadError: r.error,
    reload: r.reload,
    patch,
    setLock,
    saveError: action.error,
  };
}

/** The phone-only settings (default processing target, models over Wi-Fi only). */
export function useMobileSettings() {
  const r = useResource<MobileSettings>(loadMobile);
  const action = useAction();
  const { set } = r;
  const save = useCallback(
    (s: MobileSettings) =>
      action.run(async () => {
        set(unwrap(await ipc.commands.setMobileSettings(s)));
        return true;
      }),
    [action, set],
  );
  return {
    settings: r.data,
    loadError: r.error,
    reload: r.reload,
    save,
    saveError: action.error,
  };
}
