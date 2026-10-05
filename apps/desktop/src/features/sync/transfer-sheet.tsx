// SPDX-License-Identifier: Apache-2.0
// "Export for another device" / "Import from another device" (doc 07 §10): the
// fallback when the devices can't reach each other. The sheet only asks for a
// passphrase; the core shows the native save / open dialog and does the work,
// so the webview never sees a file path.
import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Dialog, cn, useToast } from "@ghi/ui";
import type { DeviceTransfer } from "../../bindings";
import { ipc } from "../../ipc";
import { MIN_PASSWORD, passwordIssue } from "../settings/logic";
import { Note, inputCls, useFail } from "../settings/parts";
import { transferErrorKey } from "./logic";

export type TransferMode = "export" | "import";

/** Mounted per open, so the passphrase never outlives the sheet. */
export function TransferSheet({ mode, onOpenChange }: { mode: TransferMode | null; onOpenChange: (open: boolean) => void }) {
  return mode ? <TransferDialog mode={mode} onOpenChange={onOpenChange} /> : null;
}

function TransferDialog({ mode, onOpenChange }: { mode: TransferMode; onOpenChange: (open: boolean) => void }) {
  const { t } = useTranslation();
  const { show } = useToast();
  const fail = useFail();
  const client = useQueryClient();
  const [pw, setPw] = useState("");
  const [again, setAgain] = useState("");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const exporting = mode === "export";
  // Importing only needs the passphrase the file was made with.
  const issue = exporting ? passwordIssue(pw, again) : pw.length === 0 ? "short" : null;

  const done = (d: DeviceTransfer) => {
    const key = exporting ? "settings.sync.transfer.exported" : "settings.sync.transfer.imported";
    show({ tone: "success", title: t(key, { count: d.meetings, name: d.fileName }) });
    if (!exporting) {
      if (d.refused > 0) show({ title: t("settings.sync.transfer.importedSkipped", { count: d.refused }) });
      // New meetings, folders, tags and people: every list reads again.
      void client.invalidateQueries();
    }
    onOpenChange(false);
  };

  const run = async () => {
    if (issue || busy) return;
    setBusy(true);
    setProblem(null);
    const r = exporting ? await ipc.commands.syncExportForDevice(null, pw) : await ipc.commands.syncImportFromDevice(pw, t("settings.sync.transfer.chooseTitle"));
    setBusy(false);
    if (r.status === "error") {
      const key = transferErrorKey(r.error);
      if (key) setProblem(t(`settings.sync.transfer.error.${key}`, { min: MIN_PASSWORD }));
      else fail(r.error);
      return;
    }
    if (r.data) done(r.data); // null: the file dialog was cancelled; the sheet stays for another try
  };

  return (
    <Dialog
      open
      onOpenChange={(o) => {
        if (!busy) onOpenChange(o);
      }}
      width={440}
      title={t(exporting ? "settings.sync.transfer.exportTitle" : "settings.sync.transfer.importTitle")}
      description={t(exporting ? "settings.sync.transfer.exportBody" : "settings.sync.transfer.importBody")}
      footer={
        <>
          <Button onClick={() => onOpenChange(false)} disabled={busy}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" icon={exporting ? "ios_share" : "upload_file"} type="submit" form="transfer-form" disabled={!!issue || busy}>
            {t(exporting ? "settings.sync.transfer.exportAction" : "settings.sync.transfer.importAction")}
          </Button>
        </>
      }
    >
      <form
        id="transfer-form"
        className="flex flex-col gap-2.5"
        onSubmit={(e) => {
          e.preventDefault();
          void run();
        }}
      >
        <label className="flex flex-col gap-1 text-small font-semibold">
          {t("settings.sync.transfer.passphrase")}
          <input type="password" autoComplete={exporting ? "new-password" : "current-password"} autoFocus className={cn(inputCls, "font-normal")} value={pw} onChange={(e) => setPw(e.target.value)} />
        </label>
        {exporting && (
          <label className="flex flex-col gap-1 text-small font-semibold">
            {t("settings.sync.transfer.passphraseAgain")}
            <input type="password" autoComplete="new-password" className={cn(inputCls, "font-normal")} value={again} onChange={(e) => setAgain(e.target.value)} />
          </label>
        )}
        <span role="status" className="text-small min-h-4 text-warn">
          {problem ?? (exporting && pw && issue ? (issue === "short" ? t("settings.privacy.passwordShort", { min: MIN_PASSWORD }) : again ? t("settings.privacy.passwordMismatch") : "") : "")}
        </span>
        {exporting && <Note icon="warning">{t("settings.sync.transfer.warning")}</Note>}
      </form>
    </Dialog>
  );
}
