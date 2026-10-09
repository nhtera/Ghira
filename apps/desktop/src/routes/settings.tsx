// SPDX-License-Identifier: Apache-2.0
// Settings (D11): the section list; each section lives in features/settings.
import { Link, useParams } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { Icon, cn, type IconName } from "@ghi/ui";
import { AboutSection } from "../features/settings/about-section";
import { AiSection } from "../features/settings/ai-section";
import { GeneralSection } from "../features/settings/general-section";
import { LanguagesSection } from "../features/settings/languages-section";
import { ModelsSection } from "../features/settings/models-section";
import { PrivacySection } from "../features/settings/privacy-section";
import { RecordingSection } from "../features/settings/recording-section";
import { ShortcutsSection } from "../features/settings/shortcuts-section";
import { SyncSection } from "../features/settings/sync-section";
import { TemplatesSection } from "../features/settings/templates-section";

export const SETTINGS_SECTIONS = ["general", "languages", "recording", "ai", "templates", "models", "privacy", "sync", "shortcuts", "about"] as const;
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];

const ICONS: Record<SettingsSection, IconName> = {
  general: "settings",
  languages: "translate",
  recording: "mic",
  ai: "auto_awesome",
  templates: "description",
  models: "download",
  privacy: "lock",
  sync: "sync",
  shortcuts: "keyboard_command_key",
  about: "info",
};

export function SettingsScreen() {
  const { t } = useTranslation();
  const { section } = useParams({ from: "/shell/settings/$section" });
  return (
    <div className="grid h-full min-h-0 grid-cols-[208px_minmax(0,1fr)]">
      <nav aria-label={t("settings.title")} className="flex flex-col gap-0.5 border-r border-line px-2.5 py-5">
        <h1 data-tauri-drag-region className="m-0 mb-3 ml-2.5 text-[22px] font-semibold">
          {t("settings.title")}
        </h1>
        {SETTINGS_SECTIONS.map((s) => (
          <Link
            key={s}
            to="/settings/$section"
            params={{ section: s }}
            className={cn("flex h-[34px] items-center gap-2.5 rounded-ctl px-2.5 text-[13.5px] text-muted no-underline", "hover:bg-sunk data-[status=active]:bg-surface2 data-[status=active]:font-semibold data-[status=active]:text-ink")}
          >
            <Icon name={ICONS[s]} size={18} />
            {t(`settings.sections.${s}`)}
          </Link>
        ))}
      </nav>
      <div className="min-h-0 overflow-auto px-9 pt-6 pb-12">
        <div className="max-w-[700px]">
          <h2 className="m-0 mb-1 text-[18px] font-semibold">{t(`settings.sections.${section}`)}</h2>
          {section === "general" && <GeneralSection />}
          {section === "languages" && <LanguagesSection />}
          {section === "recording" && <RecordingSection />}
          {section === "ai" && <AiSection />}
          {section === "templates" && <TemplatesSection />}
          {section === "models" && <ModelsSection />}
          {section === "privacy" && <PrivacySection />}
          {section === "sync" && <SyncSection />}
          {section === "shortcuts" && <ShortcutsSection />}
          {section === "about" && <AboutSection />}
        </div>
      </div>
    </div>
  );
}
