// SPDX-License-Identifier: Apache-2.0
// Settings → Models: one row per model with its state, one download button for
// what is missing (Wi-Fi only unless the user says otherwise this time).
import { ListRow, ListSection } from "@ghi/ui";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { MobileModelItem } from "../../bindings";
import { ipc } from "../../ipc";
import { unwrap, useAction, useResource } from "../settings/api";
import { Btn, ErrorLine, Switch } from "../settings/controls";
import { Page } from "../settings/page";
import { useMobileSettings } from "../settings/use-settings";
import { formatBytes } from "./format-bytes";

const loadStatus = async () => unwrap(await ipc.commands.modelsStatus());

export function ModelsScreen() {
  const { t, i18n } = useTranslation();
  const status = useResource(loadStatus);
  const mobile = useMobileSettings();
  const action = useAction();
  // Download progress arrives as events; the last event per model wins over the loaded status.
  const [live, setLive] = useState<Record<string, MobileModelItem>>({});

  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    void ipc
      .onMobileEvent((e) => {
        if (e.type === "modelDownload")
          setLive((m) => ({ ...m, [e.item.id]: e.item }));
      })
      .then((u) => (alive ? (off = u) : u()));
    return () => {
      alive = false;
      off?.();
    };
  }, []);

  const items = status.data?.items.map((i) => live[i.id] ?? i) ?? [];
  const downloading = items.some((i) => i.state === "downloading");
  const missing = items.filter((i) => i.state !== "ready");
  const missingBytes = missing.reduce(
    (n, i) => n + Math.max((i.sizeBytes ?? 0) - (i.receivedBytes ?? 0), 0),
    0,
  );
  const wifiOnly = mobile.settings?.modelsWifiOnly ?? true;

  const download = (wifi: boolean) =>
    void action.run(async () =>
      unwrap(await ipc.commands.modelsDownload(wifi)),
    );
  const cancel = () =>
    void action.run(async () => unwrap(await ipc.commands.modelsCancel()));

  return (
    <Page
      title={t("mobile.settings.models.title")}
      back="settings"
      error={status.error ?? mobile.loadError}
      onRetry={() => (status.reload(), mobile.reload())}
    >
      {status.data && mobile.settings && (
        <>
          <ErrorLine
            code={action.error ?? mobile.saveError}
            fallback="mobile.settings.saveFailed"
          />
          <ListSection
            header={t("mobile.settings.models.header")}
            footer={t("mobile.settings.models.footer")}
          >
            {items.map((m) => {
              const percent = m.sizeBytes
                ? Math.round(((m.receivedBytes ?? 0) / m.sizeBytes) * 100)
                : 0;
              const name = t(`mobile.settings.models.role.${m.role}`);
              return (
                <ListRow
                  key={m.id}
                  title={name}
                  value={formatBytes(m.sizeBytes, i18n.language)}
                  subtitle={
                    <>
                      <span data-model-state={m.state}>
                        {t(`mobile.settings.models.state.${m.state}`, {
                          percent,
                        })}
                      </span>
                      {m.state === "downloading" && (
                        <span
                          role="progressbar"
                          aria-label={t("mobile.settings.models.progress", {
                            name,
                          })}
                          aria-valuemin={0}
                          aria-valuemax={100}
                          aria-valuenow={percent}
                          className="mt-1 block h-1.5 overflow-hidden rounded-[3px] bg-sunk"
                        >
                          <i
                            className="block h-full bg-accent"
                            style={{ width: `${percent}%` }}
                          />
                        </span>
                      )}
                    </>
                  }
                />
              );
            })}
          </ListSection>
          <ListSection>
            <ListRow
              title={t("mobile.settings.models.wifiOnly")}
              trailing={(id) => (
                <Switch
                  checked={wifiOnly}
                  labelledBy={id}
                  onChange={(on) =>
                    void mobile.save({
                      ...mobile.settings!,
                      modelsWifiOnly: on,
                    })
                  }
                />
              )}
            />
          </ListSection>
          <div className="mx-4 flex flex-col gap-2">
            {downloading ? (
              <Btn onClick={cancel}>{t("mobile.settings.models.cancel")}</Btn>
            ) : missing.length > 0 ? (
              <>
                <Btn
                  tone="primary"
                  onClick={() => download(wifiOnly)}
                  disabled={action.busy}
                >
                  {t("mobile.settings.models.download", {
                    size: formatBytes(
                      missingBytes || status.data.missingBytes,
                      i18n.language,
                    ),
                  })}
                </Btn>
                {items.some((i) => i.state === "waitingForWifi") && (
                  <Btn onClick={() => download(false)}>
                    {t("mobile.settings.models.cellular")}
                  </Btn>
                )}
              </>
            ) : (
              <p className="text-ios-footnote m-0 text-muted">
                {t("mobile.settings.models.allReady")}
              </p>
            )}
          </div>
        </>
      )}
    </Page>
  );
}
