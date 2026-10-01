// SPDX-License-Identifier: Apache-2.0
// Settings (D11): section list; General has the theme and the app language
// (phase 9). The other sections arrive with their features.
import { Link, useParams } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { Icon, Segmented, cn, type IconName, type ThemePreference } from "@ghi/ui";
import type { Locale } from "@ghi/i18n";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { MeetingLanguage } from "../bindings";
import { ipc } from "../ipc";
import { settingsQuery } from "../shell/root-view";
import { usePrefs } from "../state/prefs";

export const SETTINGS_SECTIONS = ["general", "languages", "recording", "ai", "models", "privacy", "sync", "shortcuts"] as const;
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];

const ICONS: Record<SettingsSection, IconName> = {
  general: "settings",
  languages: "translate",
  recording: "mic",
  ai: "auto_awesome",
  models: "download",
  privacy: "lock",
  sync: "sync",
  shortcuts: "keyboard_command_key",
};

export function SettingsScreen() {
  const { t } = useTranslation();
  const { section } = useParams({ from: "/shell/settings/$section" });
  return (
    <div className="grid h-full min-h-0 grid-cols-[208px_minmax(0,1fr)]">
      <nav aria-label={t("settings.title")} className="flex flex-col gap-0.5 border-r border-line px-2.5 py-5">
        <h1 data-tauri-drag-region className="text-title m-0 mb-3 ml-2.5">
          {t("settings.title")}
        </h1>
        {SETTINGS_SECTIONS.map((s) => (
          <Link
            key={s}
            to="/settings/$section"
            params={{ section: s }}
            className={cn(
              "flex h-[34px] items-center gap-2.5 rounded-ctl px-2.5 text-[13.5px] text-muted no-underline",
              "hover:bg-sunk data-[status=active]:bg-surface2 data-[status=active]:font-semibold data-[status=active]:text-ink",
            )}
          >
            <Icon name={ICONS[s]} size={18} />
            {t(`settings.sections.${s}`)}
          </Link>
        ))}
      </nav>
      <div className="min-h-0 overflow-auto px-8 py-6">
        <h2 className="text-heading m-0 mb-4">{t(`settings.sections.${section}`)}</h2>
        {section === "general" && <General />}
      </div>
    </div>
  );
}

function General() {
  const { t } = useTranslation();
  const { theme, setTheme, language, setLanguage } = usePrefs();
  const queryClient = useQueryClient();
  const { data: settings } = useQuery(settingsQuery);
  const setMeetingLanguage = async (meetingLanguage: MeetingLanguage) => {
    const r = await ipc.commands.updateSettings({
      meetingLanguage,
      onboardingDone: null,
      detectMeetings: null,
      globalMarkShortcut: null,
      strictOffline: null,
    });
    if (r.status === "ok") queryClient.setQueryData(settingsQuery.queryKey, r.data);
  };
  return (
    <dl className="m-0 grid max-w-xl grid-cols-[minmax(0,1fr)_auto] items-center gap-x-6 gap-y-4">
      <dt className="text-body">{t("settings.general.appearance")}</dt>
      <dd className="m-0">
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
      </dd>
      <dt className="text-body">{t("settings.general.appLanguage")}</dt>
      <dd className="m-0">
        <Segmented<Locale>
          label={t("settings.general.appLanguage")}
          value={language}
          onChange={setLanguage}
          options={[
            { value: "en", label: "English" },
            { value: "vi", label: "Tiếng Việt" },
          ]}
        />
      </dd>
      <dt className="text-body">{t("onboarding.languages.title")}</dt>
      <dd className="m-0">
        {settings && (
          <Segmented<MeetingLanguage>
            label={t("onboarding.languages.title")}
            value={settings.meetingLanguage}
            onChange={(v) => void setMeetingLanguage(v)}
            options={[
              { value: "auto", label: t("onboarding.languages.both") },
              { value: "vi", label: t("onboarding.languages.vietnamese") },
              { value: "en", label: t("onboarding.languages.english") },
            ]}
          />
        )}
      </dd>
    </dl>
  );
}
