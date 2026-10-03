// SPDX-License-Identifier: Apache-2.0
// The app lock (D12) for every window. Until the lock state is known nothing
// renders. While locked (cached meeting data dropped; the core sends no
// transcript or speaker events):
// - `full` (main window, popover): only the lock screen renders;
// - `controls` (mini-recorder, detection prompt): the page renders, and shows
//   no words (it reads `useLock`), so a recording can still be paused or
//   stopped.
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { ipc } from "../ipc";
import { LockedScreen } from "../features/system-states/locked-screen";
import { useLock } from "../state/lock";

const LOCK_ERRORS = ["noAuthMethod", "noAnswer", "notConfirmed"] as const;

/** A lock command's error code in words (other errors as they are). */
export function lockErrorText(t: TFunction, error: string): string {
  const code = LOCK_ERRORS.find((c) => c === error);
  return code ? t(`system.locked.errors.${code}`) : error;
}

export function LockGate({ children, mode = "full" }: { children: ReactNode; mode?: "full" | "controls" }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const locked = useLock((s) => s.locked);
  const setLocked = (v: boolean) => useLock.setState({ locked: v });
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    // An event is newer than the first answer: a late answer doesn't override it.
    let heard = false;
    void ipc
      .onLockChanged((e) => {
        heard = true;
        setLocked(e.locked);
      })
      .then((u) => (gone ? u() : (off = u)));
    void ipc.commands.lockState().then((r) => !gone && !heard && setLocked(r.status === "ok" ? r.data : false));
    return () => {
      gone = true;
      off?.();
    };
  }, []);

  useEffect(() => {
    if (locked) client.clear();
  }, [locked, client]);

  if (locked === null) return null;
  if (locked && mode === "full") {
    const unlock = async () => {
      setError(null);
      const r = await ipc.commands.unlock(t("system.locked.reason"));
      if (r.status === "error") setError(lockErrorText(t, r.error));
      else if (!r.data) setError(t("system.locked.errors.notConfirmed"));
    };
    // "Use password" opens the same system prompt, which offers the password after Touch ID.
    return <LockedScreen onUnlock={() => void unlock()} onPassword={() => void unlock()} error={error} />;
  }
  return <>{children}</>;
}
