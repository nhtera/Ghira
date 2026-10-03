// SPDX-License-Identifier: Apache-2.0
import { useTranslation } from "react-i18next";
import { Segmented, type ThemePreference } from "@ghi/ui";
import { type Locale } from "@ghi/i18n";
import { OrganizeCard } from "../folders/organize-card";
import { usePrefs } from "../../state/prefs";
import { Card, Row } from "./parts";

export function GeneralSection() {
  const { t } = useTranslation();
  const { theme, setTheme, language, setLanguage } = usePrefs();
  return (
    <div className="flex flex-col">
      <Card>
        <Row label={t("settings.general.appearance")}>
          <Segmented<ThemePreference>
            label={t("settings.general.appearance")}
            value={theme}
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
            onChange={setLanguage}
            options={[
              { value: "en", label: "English" },
              { value: "vi", label: "Tiếng Việt" },
            ]}
          />
        </Row>
      </Card>
      <OrganizeCard />
    </div>
  );
}
