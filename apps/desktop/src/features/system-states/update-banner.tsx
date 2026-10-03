// SPDX-License-Identifier: Apache-2.0
// "An update is ready": a card in the window's bottom-left corner (D12).
// Installing restarts the app, so while recording or while a meeting is being
// processed it waits (RT-12).
import { useRouterState } from "@tanstack/react-router";
import { create } from "zustand";
import { useTranslation } from "react-i18next";
import { Button, Icon, useToast } from "@ghi/ui";
import type { SessionState } from "../../bindings";
import { ipc } from "../../ipc";
import { isActive, useLive } from "../../state/live";
import { useProcessing } from "../processing/processing-store";
import { useUpdateStatus } from "../settings/use-update-status";

/** Restarting is not allowed while a recording runs or notes are being written. */
export const updateDeferred = (state: SessionState, processing: boolean) => isActive(state) || state === "processing" || processing;

export function UpdateBanner({ version, onInstall, deferred = false, onDismiss }: { version: string; onInstall: () => void; deferred?: boolean; onDismiss?: () => void }) {
  const { t } = useTranslation();
  return (
    <div
      role="status"
      data-banner="update"
      className="fixed bottom-4 left-4 z-30 flex w-80 flex-col gap-1.5 rounded-[12px] border border-line2 bg-surface px-4 py-3.5 shadow-float"
    >
      <div className="flex items-center gap-2">
        <Icon name="system_update_alt" size={20} className="text-accent" />
        <b className="text-[14px]">{t("system.update.title", { version })}</b>
      </div>
      <span className="text-[12.5px] text-muted">{deferred ? t("system.update.deferred") : t("system.update.body")}</span>
      <div className="mt-1 flex gap-2">
        <Button size="sm" variant="primary" className="h-[30px] px-3" disabled={deferred} onClick={onInstall}>
          {t("system.update.restart")}
        </Button>
        {onDismiss && (
          <Button size="sm" className="h-[30px] px-3 font-normal" onClick={onDismiss}>
            {t("common.later")}
          </Button>
        )}
      </div>
    </div>
  );
}

/** "Later" outlives remounts (lock and unlock) but not the session. */
const useLater = create<{ later: boolean; set: () => void }>((set) => ({ later: false, set: () => set({ later: true }) }));

/** The update the core has downloaded and checked, offered until "Later" (this session only). */
export function UpdateReady() {
  const { t } = useTranslation();
  const status = useUpdateStatus();
  const state = useLive((s) => s.state);
  const processing = useProcessing((s) => Object.keys(s.meetings).length > 0);
  const { show } = useToast();
  const { later, set } = useLater();
  // The live screen's footer sits where the card goes, and an update is deferred while recording anyway.
  const onLive = useRouterState({ select: (r) => r.location.pathname.startsWith("/live") });
  // A withdrawn or too-old build has its own banner in the shell.
  if (later || onLive || !status || !status.ready || !status.available || status.runningPulled || status.reinstallNeeded) return null;
  return (
    <UpdateBanner
      version={status.available}
      deferred={updateDeferred(state, processing)}
      onInstall={async () => {
        const r = await ipc.commands.installUpdate();
        // The core refuses while it is busy (a job it knows of that this window doesn't): say so in words.
        if (r.status === "error") show({ tone: "warning", title: t("system.update.deferred") });
      }}
      onDismiss={set}
    />
  );
}
