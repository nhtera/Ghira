// SPDX-License-Identifier: Apache-2.0
import { useTranslation } from "react-i18next";
import { Segmented, type ThemePreference } from "@ghi/ui";
import { APP_NAME, type Locale } from "@ghi/i18n";
import { usePrefs } from "../../state/prefs";
import { Card, Row, SwitchRow, useSettings } from "./parts";

export function GeneralSection() {
  const { t } = useTranslation();
  const { theme, setTheme, language, setLanguage } = usePrefs();
  const { settings, patch } = useSettings();
  return (
    <Card>
      <Row label={t("settings.general.appearance")}>
        <Segmented<ThemePreference>
          label={t("settings.general.appearance")}
          value={theme}
          onChange={setTheme}
          options={[
            { value: "system", label: t("settings.general.themeSystem") },
            { value: "light", label: t("settings.general.themeLight"), icon: "light_mode" },
            { value: "dark", label: t("settings.general.themeDark"), icon: "dark_mode" },
          ]}
        />
      </Row>
      <Row label={t("settings.general.appLanguage")}>
        <Segmented<Locale>
          label={t("settings.general.appLanguage")}
          value={language}
          onChange={setLanguage}
          options={[
            { value: "en", label: "English" },
            { value: "vi", label: "Tiếng Việt" },
          ]}
        />
      </Row>
      {settings && (
        <SwitchRow
          label={t("settings.general.detectMeetings")}
          hint={t("settings.general.detectMeetingsHint", { app: APP_NAME })}
          checked={settings.detectMeetings}
          onChange={(v) => void patch({ detectMeetings: v })}
        />
      )}
    </Card>
  );
}
