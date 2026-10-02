// SPDX-License-Identifier: Apache-2.0
// Settings → Sync: not built yet, so a static card (no controls).
import { useTranslation } from "react-i18next";
import { Icon } from "@ghi/ui";
import { Card } from "./parts";

export function SyncSection() {
  const { t } = useTranslation();
  return (
    <Card title={t("settings.sync.soonTitle")} hint={t("settings.sync.soonBody")}>
      <p className="text-small m-0 flex items-center gap-2 text-muted">
        <Icon name="lock" size={16} />
        {t("settings.sync.localOnly")}
      </p>
    </Card>
  );
}
