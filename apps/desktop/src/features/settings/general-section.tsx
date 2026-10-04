// SPDX-License-Identifier: Apache-2.0
import { useTranslation } from "react-i18next";
import { Segmented, usePlatform, type ThemePreference } from "@ghi/ui";
import { APP_NAME, type Locale } from "@ghi/i18n";
import { ExportCard } from "./export-card";
import { OrganizeCard } from "../folders/organize-card";
import { usePrefs } from "../../state/prefs";
import { Card, Row, SwitchRow, bigSegCls, useSettings } from "./parts";

export function GeneralSection() {
  const { t } = useTranslation();
  const { theme, setTheme, language, setLanguage } = usePrefs();
  const context = usePlatform();
  const { settings, patch } = useSettings();
  return (
    <div className="flex flex-col">
      <Card>
        <Row label={t("settings.general.appearance")}>
          <Segmented<ThemePreference>
            label={t("settings.general.appearance")}
            value={theme}
            className={bigSegCls}
            onChange={setTheme}
            options={[
              { value: "system", label: t("settings.general.themeSystem") },
              { value: "light", label: t("settings.general.themeLight") },
              { value: "dark", label: t("settings.general.themeDark") },
            ]}
          />
        </Row>
        <Row label={t("settings.general.appLanguage")}>
          <Segmented<Locale>
            label={t("settings.general.appLanguage")}
            value={language}
            className={bigSegCls}
            onChange={setLanguage}
            options={[
              { value: "en", label: "English" },
              { value: "vi", label: "Tiếng Việt" },
            ]}
          />
        </Row>
        {settings && (
          <>
            <SwitchRow label={t("settings.general.openAtLogin", { app: APP_NAME })} hint={t("settings.general.openAtLoginHint", { app: APP_NAME })} checked={settings.openAtLogin} onChange={(v) => void patch({ openAtLogin: v })} />
            <SwitchRow label={t("settings.general.showInTray", { app: APP_NAME, context })} checked={settings.showInMenuBar} onChange={(v) => void patch({ showInMenuBar: v })} />
          </>
        )}
      </Card>
      <ExportCard />
      <OrganizeCard />
    </div>
  );
}
