// SPDX-License-Identifier: Apache-2.0
import { useTranslation } from "react-i18next";
import { APP_NAME } from "@ghi/i18n";
import { Button } from "@ghi/ui";
import { ipc } from "../../ipc";
import { UpdatesCard } from "./updates-card";
import { LicensesList } from "./licenses-list";
import { Card } from "./parts";

export function AboutSection() {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col">
      <Card title={APP_NAME} hint={t("settings.about.promise")} />
      <UpdatesCard />
      <Card title={t("settings.about.diagnostics")} hint={t("settings.about.diagnosticsHint")}>
        <div>
          <Button size="sm" icon="folder" onClick={() => void ipc.commands.revealDiagnostics()}>
            {t("settings.about.revealReports")}
          </Button>
        </div>
      </Card>
      <Card title={t("settings.about.licenses")} hint={t("settings.about.licensesHint", { app: APP_NAME })}>
        <LicensesList />
      </Card>
    </div>
  );
}
