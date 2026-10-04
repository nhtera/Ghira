// SPDX-License-Identifier: Apache-2.0
// Where exports go: the export folder and the Obsidian vault, each by name
// (never a path) with a native folder dialog to change it.
import { useTranslation } from "react-i18next";
import { Button } from "@ghi/ui";
import { useExportDestination, useObsidianVault } from "../export/destination";
import { Card, Row, useFail } from "./parts";

const EXPORT_ROW = "export-folder";
const VAULT_ROW = "obsidian-vault";

export function ExportCard() {
  const { t } = useTranslation();
  const fail = useFail();
  const dest = useExportDestination(true, t("settings.general.exportFolder"));
  const vault = useObsidianVault(t("settings.general.obsidianVault"));
  const row = (id: string, label: string, f: { folder: string | null; change: () => Promise<string | null>; failed: boolean; loading: boolean; retry: () => void }) => (
    <Row label={label}>
      {f.failed ? (
        <>
          <span role="alert" className="text-small text-rec-ink" data-testid={`${id}-error`}>
            {t("settings.general.folderFailed")}
          </span>
          <Button size="sm" onClick={f.retry}>
            {t("common.tryAgain")}
          </Button>
        </>
      ) : (
        <>
          <span className="text-small text-muted" data-testid={`${id}-name`}>
            {f.loading ? "" : (f.folder ?? t("settings.general.folderNone"))}
          </span>
          <Button size="sm" aria-label={`${label}: ${t("export.changeDestination")}`} onClick={() => void f.change().then((e) => e && fail(e))}>
            {t("export.changeDestination")}
          </Button>
        </>
      )}
    </Row>
  );
  return (
    <Card title={t("settings.general.exportTitle")}>
      {row(EXPORT_ROW, t("settings.general.exportFolder"), dest)}
      {row(VAULT_ROW, t("settings.general.obsidianVault"), vault)}
    </Card>
  );
}
