// SPDX-License-Identifier: Apache-2.0
// Settings → Cloud notes: provider and model, hide-personal-data default, and
// the API key. The key goes straight to the Keychain through the command and
// is never read back (only "saved" is known).
import { ListRow, ListSection } from "@ghi/ui";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ipc } from "../../ipc";
import { unwrap, useAction, useResource } from "../../features/settings/api";
import { Btn, ChoiceRow, ErrorLine, Field, Switch } from "../../features/settings/controls";
import { Page } from "../../features/settings/page";
import { providerName } from "../../features/settings/providers";
import { useAppSettings } from "../../features/settings/use-settings";

const loadModels = async () => ipc.commands.cloudModels();
const loadKeys = async () => unwrap(await ipc.commands.cloudKeys());

export function CloudScreen() {
  const { t } = useTranslation();
  const app = useAppSettings();
  const models = useResource(loadModels);
  const keys = useResource(loadKeys);
  const action = useAction();
  const [key, setKey] = useState("");
  const [saved, setSaved] = useState(false);

  const s = app.settings;
  const providers = [...new Set((models.data ?? []).map((m) => m.provider))];
  const provider = s?.cloudProvider ?? "";
  const providerModels = (models.data ?? []).filter((m) => m.provider === provider);
  const stored = keys.data?.find((k) => k.provider === provider)?.stored ?? false;

  const pick = (id: string) => {
    setKey("");
    setSaved(false);
    const first = (models.data ?? []).find((m) => m.provider === id)?.model ?? "";
    void app.patch({ cloudProvider: id, cloudModel: id ? first : "" });
  };
  const saveKey = () =>
    void action.run(async () => {
      unwrap(await ipc.commands.setCloudKey(provider, key));
      setKey("");
      setSaved(true);
      keys.reload();
    });
  const removeKey = () =>
    void action.run(async () => {
      unwrap(await ipc.commands.deleteCloudKey(provider));
      setSaved(false);
      keys.reload();
    });

  return (
    <Page title={t("mobile.settings.cloud.title")} back="settings" error={app.loadError ?? keys.error ?? models.error} onRetry={() => (app.reload(), keys.reload(), models.reload())}>
      {s && models.data && keys.data && (
        <>
          <ErrorLine code={action.error ?? app.saveError} fallback="mobile.settings.saveFailed" />
          <ListSection header={t("mobile.settings.cloud.header")} footer={t("mobile.settings.cloud.footer")}>
            <ChoiceRow title={t("mobile.settings.cloud.none")} selected={provider === ""} onPress={() => pick("")} />
            {providers.map((id) => (
              <ChoiceRow key={id} title={providerName(id)} selected={provider === id} onPress={() => pick(id)} />
            ))}
          </ListSection>
          {providers.length === 0 && <p className="text-ios-footnote mx-4 text-muted">{t("mobile.settings.cloud.noProviders")}</p>}
          {provider && (
            <>
              <ListSection header={t("mobile.settings.cloud.model")}>
                {providerModels.map((m) => (
                  <ChoiceRow key={m.model} title={m.model} selected={s.cloudModel === m.model} onPress={() => void app.patch({ cloudModel: m.model })} />
                ))}
              </ListSection>
              <ListSection>
                <ListRow
                  title={t("mobile.settings.cloud.redactDefault")}
                  trailing={(id) => <Switch checked={s.cloudRedact} labelledBy={id} onChange={(on) => void app.patch({ cloudRedact: on })} />}
                />
              </ListSection>
              <ListSection header={t("mobile.settings.cloud.keyHeader")} footer={t("mobile.settings.cloud.keyFooter")}>
                <ListRow title={providerName(provider)} value={stored ? t("mobile.settings.status.keyStored") : t("mobile.settings.status.noKey")} />
              </ListSection>
              <div className="mx-4 flex flex-col gap-3">
                <Field
                  id="cloud-key"
                  label={t("mobile.settings.cloud.keyField", { provider: providerName(provider) })}
                  type="password"
                  autoComplete="off"
                  autoCorrect="off"
                  autoCapitalize="none"
                  spellCheck={false}
                  value={key}
                  onChange={(e) => setKey(e.target.value)}
                />
                <Btn tone="primary" onClick={saveKey} disabled={!key.trim() || action.busy}>
                  {t("mobile.settings.cloud.keySave")}
                </Btn>
                {saved && (
                  <p role="status" className="text-ios-footnote m-0 text-accent">
                    {t("mobile.settings.cloud.keySaved")}
                  </p>
                )}
                {stored && (
                  <Btn tone="danger" onClick={removeKey} disabled={action.busy}>
                    {t("mobile.settings.cloud.keyDelete")}
                  </Btn>
                )}
              </div>
            </>
          )}
        </>
      )}
    </Page>
  );
}
