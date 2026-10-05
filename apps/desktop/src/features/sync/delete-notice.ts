// SPDX-License-Identifier: Apache-2.0
// After a meeting is deleted here: "Deleted here. It will be deleted on
// <device> at the next sync.", then "Deleted on all devices" once a session
// ends with nothing pending (doc 07 §7.9). Says nothing when sync is off or
// nobody is paired.
import type { ToastInput } from "@ghi/ui";
import type { TFunction } from "i18next";
import { ipc } from "../../ipc";

/** The wait for a sync is not unbounded: the listener goes away after this. */
const WATCH_MS = 30 * 60_000;

export async function announceDeleted(show: (t: ToastInput) => void, t: TFunction): Promise<void> {
  const r = await ipc.commands.syncStatus();
  if (r.status !== "ok" || !r.data.enabled || r.data.paired.length === 0) return;
  const device = r.data.paired.map((d) => d.name).join(", ");
  show({ title: t("settings.sync.deletedHere", { device }) });
  const state: { done: boolean; off?: () => void } = { done: false };
  const finish = () => {
    state.done = true;
    state.off?.();
  };
  state.off = await ipc.onSyncEvent((e) => {
    if (state.done || e.type !== "progress" || e.pending !== 0) return;
    finish();
    show({ tone: "success", title: t("settings.sync.deletedEverywhere") });
  });
  if (state.done) state.off();
  else setTimeout(finish, WATCH_MS);
}
