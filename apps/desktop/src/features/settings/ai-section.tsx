// SPDX-License-Identifier: Apache-2.0
// Settings → AI: local vs cloud, provider keys, the send sheet's defaults and
// the log of everything that left the device. A saved key is never shown
// again (only "Key saved" + Remove): the input is cleared the moment it is sent.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, Icon, usePlatform } from "@ghi/ui";
import { APP_NAME, formatDate, formatTime, type Locale } from "@ghi/i18n";
import { ipc } from "../../ipc";
import { providerName } from "../cloud-sheet/provider-names";
import { Card, Note, Row, SwitchRow, inputCls, useFail, useSettings } from "./parts";

const keysKey = ["cloud-keys"] as const;

export function AiSection() {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col gap-4">
      <div className="grid max-w-2xl grid-cols-2 gap-3 max-sm:grid-cols-1">
        <Card title={t("settings.ai.local.title")} hint={t("settings.ai.local.body")} className="border-accent bg-accent-soft">
          <span className="text-small inline-flex items-center gap-1.5 font-semibold text-accent">
            <Icon name="check_circle" size={16} />
            {t("settings.ai.default")}
          </span>
        </Card>
        <Card title={t("settings.ai.cloud.title")} hint={t("settings.ai.cloud.body")} />
      </div>
      <KeysCard />
      <DefaultsCard />
      <RequestLog />
    </div>
  );
}

function KeysCard() {
  const { t } = useTranslation();
  const context = usePlatform();
  const { data: keys } = useQuery({
    queryKey: keysKey,
    queryFn: async () => {
      const r = await ipc.commands.cloudKeys();
      if (r.status === "error") throw new Error(r.error);
      return r.data;
    },
  });
  return (
    <Card title={t("settings.ai.keys.title")} hint={t("settings.ai.keys.hint", { context })}>
      <ul className="m-0 flex list-none flex-col gap-3 p-0">
        {(keys ?? []).map((k) => (
          <li key={k.provider}>
            <KeyRow provider={k.provider} stored={k.stored} />
          </li>
        ))}
      </ul>
    </Card>
  );
}

function KeyRow({ provider, stored }: { provider: string; stored: boolean }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const fail = useFail();
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const name = providerName(provider);
  const refresh = () => queryClient.invalidateQueries({ queryKey: keysKey });
  const save = async () => {
    if (!key.trim()) return;
    setBusy(true);
    const r = await ipc.commands.setCloudKey(provider, key);
    setKey("");
    setBusy(false);
    if (r.status === "error") fail(r.error);
    await refresh();
  };
  const remove = async () => {
    const r = await ipc.commands.deleteCloudKey(provider);
    if (r.status === "error") fail(r.error);
    await refresh();
  };
  return (
    <div className="flex items-center justify-between gap-3" data-testid={`key-${provider}`}>
      <b className="text-body w-24 shrink-0 font-semibold">{name}</b>
      {stored ? (
        <>
          <span className="text-small flex flex-1 items-center gap-1.5 text-accent">
            <Icon name="check_circle" size={16} />
            {t("settings.ai.keys.saved")}
          </span>
          <Button size="sm" onClick={() => void remove()} aria-label={t("settings.ai.keys.removeFor", { provider: name })}>
            {t("common.remove")}
          </Button>
        </>
      ) : (
        <form
          className="flex flex-1 items-center gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            void save();
          }}
        >
          <input
            type="password"
            autoComplete="off"
            spellCheck={false}
            className={`${inputCls} flex-1`}
            value={key}
            aria-label={t("settings.ai.keys.inputFor", { provider: name })}
            placeholder={t("settings.ai.keys.placeholder")}
            onChange={(e) => setKey(e.target.value)}
          />
          <Button type="submit" size="sm" variant="primary" disabled={!key.trim() || busy} aria-label={t("settings.ai.keys.saveFor", { provider: name })}>
            {t("common.save")}
          </Button>
        </form>
      )}
    </div>
  );
}

function DefaultsCard() {
  const { t } = useTranslation();
  const { settings, patch } = useSettings();
  const { data: keys } = useQuery({
    queryKey: keysKey,
    queryFn: async () => {
      const r = await ipc.commands.cloudKeys();
      if (r.status === "error") throw new Error(r.error);
      return r.data;
    },
  });
  const { data: models } = useQuery({ queryKey: ["cloud-models"], queryFn: () => ipc.commands.cloudModels() });
  if (!settings) return null;
  const withKey = new Set((keys ?? []).filter((k) => k.stored).map((k) => k.provider));
  const providers = [...new Set((models ?? []).map((m) => m.provider))].filter((p) => withKey.has(p));
  const forProvider = (models ?? []).filter((m) => m.provider === settings.cloudProvider);
  return (
    <Card title={t("settings.ai.defaults.title")} hint={t("settings.ai.defaults.hint")}>
      {providers.length === 0 ? (
        <Note>{t("settings.ai.defaults.needKey")}</Note>
      ) : (
        <>
          <Row label={t("cloud.provider")}>
            <select
              className={inputCls}
              aria-label={t("cloud.provider")}
              value={withKey.has(settings.cloudProvider) ? settings.cloudProvider : ""}
              onChange={(e) => {
                const provider = e.target.value;
                void patch({ cloudProvider: provider, cloudModel: (models ?? []).find((m) => m.provider === provider)?.model ?? "" });
              }}
            >
              <option value="">{t("settings.ai.defaults.none")}</option>
              {providers.map((p) => (
                <option key={p} value={p}>
                  {providerName(p)}
                </option>
              ))}
            </select>
          </Row>
          {withKey.has(settings.cloudProvider) && (
            <Row label={t("settings.ai.defaults.model")}>
              <select className={inputCls} aria-label={t("settings.ai.defaults.model")} value={settings.cloudModel} onChange={(e) => void patch({ cloudModel: e.target.value })}>
                {forProvider.map((m) => (
                  <option key={m.model} value={m.model}>
                    {m.model}
                  </option>
                ))}
              </select>
            </Row>
          )}
        </>
      )}
      <SwitchRow label={t("settings.ai.redactDefault")} hint={t("settings.ai.redactDefaultHint")} checked={settings.cloudRedact} onChange={(v) => void patch({ cloudRedact: v })} />
      <Note icon="info">{t("settings.ai.alwaysPreview", { app: APP_NAME })}</Note>
    </Card>
  );
}

function RequestLog() {
  const { t, i18n } = useTranslation();
  const lang = i18n.language as Locale;
  const nf = new Intl.NumberFormat(lang === "vi" ? "vi-VN" : "en-US");
  const { data: log } = useQuery({
    queryKey: ["cloud-log"],
    queryFn: async () => {
      const r = await ipc.commands.cloudRequestLog(100);
      if (r.status === "error") throw new Error(r.error);
      return r.data;
    },
  });
  return (
    <Card title={t("settings.ai.log.title")} hint={t("settings.ai.log.subtitle")} className="max-w-3xl">
      {!log || log.length === 0 ? (
        <p className="text-small m-0 text-muted" data-testid="log-empty">
          {t("settings.ai.log.empty")}
        </p>
      ) : (
        <div className="overflow-x-auto">
          <table className="text-small w-full border-collapse text-left">
            <thead className="text-muted">
              <tr>
                <th className="py-1 pr-3 font-medium">{t("settings.ai.log.columns.time")}</th>
                <th className="py-1 pr-3 font-medium">{t("settings.ai.log.columns.meeting")}</th>
                <th className="py-1 pr-3 font-medium">{t("settings.ai.log.columns.provider")}</th>
                <th className="py-1 text-right font-medium">{t("settings.ai.log.columns.tokens")}</th>
              </tr>
            </thead>
            <tbody>
              {log.map((e, i) => (
                <tr key={`${e.at}-${i}`} className="border-t border-line">
                  <td className="py-1.5 pr-3 whitespace-nowrap">{e.at != null ? `${formatDate(new Date(e.at), lang)} ${formatTime(new Date(e.at), lang)}` : ""}</td>
                  <td className="py-1.5 pr-3">{e.meetingTitle}</td>
                  <td className="py-1.5 pr-3">
                    {providerName(e.provider)} · {e.model}
                  </td>
                  <td className="py-1.5 text-right tabular-nums">{nf.format((e.tokensIn ?? 0) + (e.tokensOut ?? 0))}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Card>
  );
}
