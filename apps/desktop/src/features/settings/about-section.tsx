// SPDX-License-Identifier: Apache-2.0
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { APP_NAME } from "@ghi/i18n";
import { ipc } from "../../ipc";
import { LicensesList } from "./licenses-list";
import { Card } from "./parts";

export function AboutSection() {
  const { t } = useTranslation();
  const { data: version } = useQuery({ queryKey: ["app-version"], queryFn: () => ipc.commands.appVersion() });
  return (
    <div className="flex max-w-2xl flex-col gap-4">
      <Card title={APP_NAME} hint={t("settings.about.promise")}>
        {version && (
          <p className="text-small m-0 text-muted" data-testid="app-version">
            {t("settings.about.version", { app: version.app, core: version.core })}
          </p>
        )}
      </Card>
      <Card title={t("settings.about.licenses")} hint={t("settings.about.licensesHint", { app: APP_NAME })}>
        <LicensesList />
      </Card>
    </div>
  );
}
