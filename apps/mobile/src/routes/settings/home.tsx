// SPDX-License-Identifier: Apache-2.0
// Settings home: the inset grouped list. Processing target and meeting
// language are picked here; everything else opens a screen.
import { ListRow, ListSection } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import type { MeetingLanguage } from "../../bindings";
import { ipc } from "../../ipc";
import { useGo } from "../../features/settings/go";
import { unwrap, useResource } from "../../features/settings/api";
import { ChoiceRow, ErrorLine } from "../../features/settings/controls";
import { Page } from "../../features/settings/page";
import { useAppSettings, useMobileSettings } from "../../features/settings/use-settings";

const LANGUAGES: MeetingLanguage[] = ["auto", "en", "vi"];
const loadModels = async () => unwrap(await ipc.commands.modelsStatus());
const loadVoice = async () => unwrap(await ipc.commands.voiceStatus());

export function SettingsHome() {
  const { t } = useTranslation();
  const go = useGo();
  const app = useAppSettings();
  const mobile = useMobileSettings();
  const models = useResource(loadModels);
  const voice = useResource(loadVoice);

  const allReady = models.data ? models.data.items.every((m) => m.state === "ready") : undefined;
  const modelsValue = allReady === undefined ? undefined : allReady ? t("mobile.settings.status.ready") : t("mobile.settings.status.needsDownload");
  const voiceValue = voice.data ? (voice.data.meProfile ? t("mobile.settings.status.set") : t("mobile.settings.status.notSet")) : undefined;
  const failed = app.loadError ?? mobile.loadError;

  return (
    <Page title={t("mobile.settings.title")} error={failed} onRetry={() => (app.reload(), mobile.reload())}>
      {app.settings && mobile.settings && (
        <>
          <ErrorLine code={app.saveError ?? mobile.saveError} fallback="mobile.settings.saveFailed" />
          <ListSection header={t("mobile.target.finalOn")} footer={t("mobile.target.hintPhone")}>
            <ChoiceRow
              title={t("mobile.target.phone")}
              selected={mobile.settings.defaultTarget === "phone"}
              onPress={() => void mobile.save({ ...mobile.settings!, defaultTarget: "phone" })}
            />
            <ChoiceRow title={t("mobile.target.desktop")} subtitle={t("mobile.settings.desktopLater")} selected={false} disabled onPress={() => undefined} />
          </ListSection>
          <ListSection header={t("mobile.settings.language.header")} footer={t("mobile.settings.language.footer")}>
            {LANGUAGES.map((l) => (
              <ChoiceRow key={l} title={t(`mobile.settings.language.${l}`)} selected={app.settings!.meetingLanguage === l} onPress={() => void app.patch({ meetingLanguage: l })} />
            ))}
          </ListSection>
          <ListSection header={t("mobile.settings.section.device")}>
            <ListRow title={t("mobile.settings.rows.models")} value={modelsValue} chevron onPress={() => go("/settings/models")} />
            <ListRow title={t("mobile.settings.rows.voice")} value={voiceValue} chevron onPress={() => go("/settings/voice")} />
          </ListSection>
          <ListSection header={t("mobile.settings.section.privacy")}>
            <ListRow title={t("mobile.settings.rows.privacy")} value={app.settings.appLock ? t("mobile.settings.status.on") : undefined} chevron onPress={() => go("/settings/privacy")} />
            <ListRow title={t("mobile.settings.rows.cloud")} chevron onPress={() => go("/settings/cloud")} />
            <ListRow title={t("mobile.settings.rows.consent")} chevron onPress={() => go("/settings/consent")} />
          </ListSection>
          <ListSection>
            <ListRow title={t("mobile.settings.rows.about")} chevron onPress={() => go("/settings/about")} />
          </ListSection>
        </>
      )}
    </Page>
  );
}
