// SPDX-License-Identifier: Apache-2.0
// Settings → About: versions and the way into the open-source licenses we ship.
import { ListRow, ListSection } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import { ipc } from "../../ipc";
import { useResource } from "../../features/settings/api";
import { useGo } from "../../features/settings/go";
import { Page } from "../../features/settings/page";

const loadVersion = () => ipc.commands.appVersion();

export function AboutScreen() {
  const { t } = useTranslation();
  const go = useGo();
  const v = useResource(loadVersion);
  return (
    <Page title={t("mobile.settings.about.title")} back="settings" error={v.error} onRetry={v.reload}>
      {v.data && (
        <>
          <ListSection footer={t("mobile.settings.about.licence")}>
            <ListRow title={t("mobile.settings.about.version")} value={v.data.app} />
            <ListRow title={t("mobile.settings.about.core")} value={v.data.core} />
          </ListSection>
          <ListSection footer={t("mobile.settings.about.licencesFooter")}>
            <ListRow title={t("mobile.settings.about.licencesHeader")} chevron onPress={() => go("/settings/about/licenses")} />
          </ListSection>
        </>
      )}
    </Page>
  );
}
