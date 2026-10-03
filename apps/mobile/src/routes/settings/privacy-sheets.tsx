// SPDX-License-Identifier: Apache-2.0
// The two sheets of Settings → Privacy: export everything (password-sealed
// archive, handed to the system share sheet) and delete everything (typed
// confirmation; Rust checks the phrase again).
import { Sheet } from "@ghi/ui";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ipc } from "../../ipc";
import { useGo } from "../../features/settings/go";
import { unwrap, useAction } from "../../features/settings/api";
import { Btn, ErrorLine, Field } from "../../features/settings/controls";
import { clearRecentSearches } from "../../features/search/recent";
import { isDeletePhrase } from "../../features/settings/fold";

const MIN_PASSWORD = 8;

type SheetProps = { open: boolean; onOpenChange: (open: boolean) => void };

function useSheetLabels() {
  const { t } = useTranslation();
  return { closeLabel: t("mobile.sheet.close"), handleLabel: t("mobile.sheet.handle") };
}

export function ExportSheet({ open, onOpenChange }: SheetProps) {
  const { t } = useTranslation();
  const labels = useSheetLabels();
  const [password, setPassword] = useState("");
  const [done, setDone] = useState(false);
  const action = useAction();
  const ready = password.length >= MIN_PASSWORD;

  const close = (o: boolean) => {
    if (!o) {
      setPassword("");
      setDone(false);
      action.clear();
    }
    onOpenChange(o);
  };
  const share = () =>
    void action.run(async () => {
      unwrap(await ipc.commands.privacyExportAllShare(password));
      setDone(true);
      setPassword("");
    });

  return (
    <Sheet
      open={open}
      onOpenChange={close}
      title={t("mobile.privacy.export.title")}
      description={t("mobile.privacy.export.body")}
      dismissible={!action.busy}
      {...labels}
      footer={
        <Btn tone="primary" onClick={share} disabled={!ready || action.busy}>
          {action.busy ? t("mobile.privacy.export.working") : t("mobile.privacy.export.confirm")}
        </Btn>
      }
    >
      <Field
        id="export-password"
        label={t("mobile.privacy.export.password")}
        type="password"
        autoComplete="off"
        autoCorrect="off"
        autoCapitalize="none"
        spellCheck={false}
        value={password}
        onChange={(e) => setPassword(e.target.value)}
        aria-describedby="export-password-hint"
        className="w-full"
      />
      <p id="export-password-hint" className="text-ios-footnote m-0 mt-1 px-1 text-muted">
        {t("mobile.privacy.export.passwordHint")}
      </p>
      <ErrorLine code={action.error} />
      {done && (
        <p role="status" className="text-ios-footnote m-0 mt-2 px-1 text-accent">
          {t("mobile.privacy.export.done")}
        </p>
      )}
    </Sheet>
  );
}

export function DeleteAllSheet({ open, onOpenChange }: SheetProps) {
  const { t } = useTranslation();
  const labels = useSheetLabels();
  const go = useGo();
  const [typed, setTyped] = useState("");
  const action = useAction();
  const phrase = t("mobile.privacy.deleteAll.phrase");

  const close = (o: boolean) => {
    if (!o) {
      setTyped("");
      action.clear();
    }
    onOpenChange(o);
  };
  const wipe = () =>
    void action.run(async () => {
      unwrap(await ipc.commands.privacyDeleteAll(typed));
      onOpenChange(false);
      // Recent searches are words from meetings: they go too.
      clearRecentSearches();
      // Back to first launch: nothing is left to show.
      go("/onboarding", { replace: true });
    });

  return (
    <Sheet
      open={open}
      onOpenChange={close}
      title={t("mobile.privacy.deleteAll.title")}
      description={t("mobile.privacy.deleteAll.body")}
      dismissible={!action.busy}
      {...labels}
      footer={
        <Btn tone="danger" onClick={wipe} disabled={!isDeletePhrase(typed) || action.busy}>
          {t("mobile.privacy.deleteAll.confirm")}
        </Btn>
      }
    >
      <Field
        id="delete-confirm"
        label={t("mobile.privacy.deleteAll.typePrompt", { phrase })}
        autoComplete="off"
        autoCorrect="off"
        autoCapitalize="characters"
        spellCheck={false}
        value={typed}
        onChange={(e) => setTyped(e.target.value)}
        className="w-full"
      />
      <ErrorLine code={action.error} />
    </Sheet>
  );
}
