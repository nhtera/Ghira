// SPDX-License-Identifier: Apache-2.0
// Quitting while recording asks first (Rust holds the exit and sends
// `quitRequested`); stopping saves the recording, and its jobs finish at the
// next launch.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Dialog, useToast } from "@ghi/ui";
import { ipc } from "../ipc";

export function QuitDialog() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const { show } = useToast();
  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    void ipc.onQuitRequested(() => setOpen(true)).then((u) => (alive ? (off = u) : u()));
    return () => {
      alive = false;
      off?.();
    };
  }, []);
  return (
    <Dialog
      open={open}
      onOpenChange={setOpen}
      title={t("system.quit.title")}
      description={t("system.quit.body")}
      footer={
        <>
          <Button onClick={() => setOpen(false)}>{t("system.quit.keepRecording")}</Button>
          <Button
            variant="danger"
            disabled={busy}
            onClick={() => {
              setBusy(true);
              void ipc.commands.quitApp(true).then((r) => {
                setBusy(false);
                if (r.status === "error") show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
              });
            }}
          >
            {t("system.quit.stopAndQuit")}
          </Button>
        </>
      }
    />
  );
}
