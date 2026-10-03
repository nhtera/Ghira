// SPDX-License-Identifier: Apache-2.0
// Core `error` events the live view doesn't own: a capture device taken by
// another app, a permission turned off, the store not opening. Repeats collapse
// into one notice; a dismissed one stays dismissed. A storage error blocks
// (nothing can be saved), with a retry that reloads the window.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Icon, usePlatform, useToast } from "@ghi/ui";
import type { ErrorKind } from "../../bindings";
import { ipc } from "../../ipc";
import { BANNER_ACTION, SystemBanner } from "./system-banner";

export type CoreError = { key: string; kind: ErrorKind; message: string; meeting: string | null };
const SHOWN: ErrorKind[] = ["capture", "storage", "permission"];

/** The shown errors, deduplicated by kind + message. */
export function useCoreErrors() {
  const [errors, setErrors] = useState<CoreError[]>([]);
  const [dismissed, setDismissed] = useState<string[]>([]);
  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    void ipc
      .onCoreEvent((env) => {
        const e = env.event;
        if (e.type !== "error" || !SHOWN.includes(e.kind)) return;
        const key = `${e.kind}|${e.meeting ?? ""}|${e.message}`;
        setErrors((prev) => (prev.some((x) => x.key === key) ? prev : [...prev, { key, kind: e.kind, message: e.message, meeting: e.meeting }]));
      })
      .then((u) => (alive ? (off = u) : u()));
    return () => {
      alive = false;
      off?.();
    };
  }, []);
  return {
    errors: errors.filter((e) => !dismissed.includes(e.key)),
    dismiss: (key: string) => setDismissed((d) => [...d, key]),
  };
}

/**
 * The core's text for another app holding the mic is not translated, so this
 * guesses from the wording. It is a heuristic: a miss only costs us the
 * translated copy, because anything else falls back to the generic capture
 * banner with the core's message. Replace it when capture errors get a reason.
 */
export const isMicTaken = (message: string) => /exclusive|in use|busy|taken|another app/i.test(message);

export function StorageErrorScreen({ message, onRetry }: { message: string; onRetry: () => void }) {
  const { t } = useTranslation();
  return (
    <div role="alert" data-banner="storage" className="fixed inset-0 z-50 grid place-items-center bg-surface p-8">
      <div className="flex max-w-[460px] flex-col items-start gap-3">
        <Icon name="error" size={40} className="text-rec-ink" />
        <h1 className="text-title m-0">{t("system.storage.title")}</h1>
        <p className="m-0 text-[14px] leading-relaxed text-muted">{t("system.storage.body")}</p>
        {message && <p className="text-mono m-0 text-[12px] text-faint">{message}</p>}
        <Button variant="primary" size="lg" autoFocus onClick={onRetry}>
          {t("common.tryAgain")}
        </Button>
      </div>
    </div>
  );
}

export function CoreErrorBanners() {
  const { t } = useTranslation();
  const context = usePlatform();
  const { show } = useToast();
  const { errors, dismiss } = useCoreErrors();
  // Only "the store couldn't open" (no meeting) blocks. A write failing during a
  // recording also arrives as `storage`, with the meeting: that is a banner.
  const storage = errors.find((e) => e.kind === "storage" && e.meeting == null);
  if (storage) return <StorageErrorScreen message={storage.message} onRetry={() => window.location.reload()} />;

  const openAudioSettings = async () => {
    const r = await ipc.commands.openPrivacySettings("systemAudio");
    if (r.status === "error") show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
  };
  return (
    <>
      {errors.map((e) =>
        e.kind === "permission" ? (
          <SystemBanner
            key={e.key}
            id="permission"
            icon="warning"
            onDismiss={() => dismiss(e.key)}
            actions={
              <Button size="sm" variant="ghost" className={BANNER_ACTION} onClick={() => void openAudioSettings()}>
                {t(`common.openSystemSettings_${context}`)}
              </Button>
            }
          >
            {t("system.permissionLost")}
          </SystemBanner>
        ) : e.kind === "storage" ? (
          <SystemBanner key={e.key} id="storage-write" tone="rec" icon="error" onDismiss={() => dismiss(e.key)}>
            {t("system.storageWrite")} {e.message}
          </SystemBanner>
        ) : (
          <SystemBanner key={e.key} id="capture" assertive icon="mic_off" onDismiss={() => dismiss(e.key)}>
            {isMicTaken(e.message) ? t("system.micTaken") : e.message}
          </SystemBanner>
        ),
      )}
    </>
  );
}
