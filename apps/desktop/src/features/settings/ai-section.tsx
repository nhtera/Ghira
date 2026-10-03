// SPDX-License-Identifier: Apache-2.0
// Settings → AI: local vs cloud, provider keys, the send sheet's defaults and
// the log of everything that left the device. A saved key is never shown
// again (only "Key saved" + Remove): the input is cleared the moment it is sent.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button, Icon, cn, usePlatform, type IconName } from "@ghi/ui";
import { APP_NAME, formatDate, formatTime, type Locale } from "@ghi/i18n";
import { ipc } from "../../ipc";
import { providerName } from "../cloud-sheet/provider-names";
import { Card, Note, Row, Switch, SwitchRow, inputCls, useFail, useSettings } from "./parts";

const keysKey = ["cloud-keys"] as const;

export function AiSection() {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col">
      <div className="grid gap-2 pt-3">
        <ModeCard selected icon="laptop_mac" title={t("settings.ai.local.title")} body={t("settings.ai.local.body")} badge={t("settings.ai.default")} />
        <ModeCard icon="cloud" title={t("settings.ai.cloud.title")} body={t("settings.ai.cloud.body")} badge={t("settings.ai.perMeeting")} />
      </div>
      <KeysCard />
      <DefaultsCard />
      <Row label={t("cloud.audioNever")}>
        <Icon name="lock" size={18} className="text-accent" />
        <Switch checked onChange={() => {}} label={t("cloud.audioNever")} disabled />
      </Row>
      <RulesCard />
      <RequestLog />
    </div>
  );
}

/** The AI mode as the design draws it. Cloud is chosen per meeting, so these cards show the default and are not controls. */
function ModeCard({ selected, icon, title, body, badge }: { selected?: boolean; icon: IconName; title: string; body: string; badge: string }) {
  return (
    <div className={cn("grid grid-cols-[24px_24px_minmax(0,1fr)] items-start gap-x-3 rounded-xl border-[1.5px] bg-surface px-4 py-3.5", selected ? "border-accent" : "border-ctl")}>
      {selected ? <Icon name="check_circle" size={20} className="text-accent" /> : <span />}
      <Icon name={icon} size={20} className="text-muted" />
      <span>
        <b className="text-[14px]">{title}</b>
        <span className={cn("ml-1 text-[11.5px] font-semibold", selected ? "text-accent" : "text-muted")}>{badge}</span>
        <span className="mt-0.5 block text-[12.5px] leading-normal text-muted">{body}</span>
      </span>
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
      <ul className="m-0 flex list-none flex-col p-0">
        {(keys ?? []).map((k) => (
          <li key={k.provider} className="border-b border-line py-3">
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
      <b className="w-24 shrink-0 text-[14px] font-semibold">{name}</b>
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
  const { data: models } = useQuery({
    queryKey: ["cloud-models"],
    queryFn: () => ipc.commands.cloudModels(),
  });
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
                void patch({
                  cloudProvider: provider,
                  cloudModel: (models ?? []).find((m) => m.provider === provider)?.model ?? "",
                });
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
      <Note icon="info">{t("settings.ai.alwaysPreview", { app: APP_NAME })}</Note>
    </Card>
  );
}

function RulesCard() {
  const { t } = useTranslation();
  const { settings, patch } = useSettings();
  if (!settings) return null;
  return (
    <Card title={t("settings.ai.rules.title")}>
      <SwitchRow label={t("settings.ai.redactDefault")} hint={t("settings.ai.redactDefaultHint")} checked={settings.cloudRedact} onChange={(v) => void patch({ cloudRedact: v })} />
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
    <Card title={t("settings.ai.log.title")} hint={t("settings.ai.log.subtitle")}>
      {!log || log.length === 0 ? (
        <p className="text-small m-0 text-muted" data-testid="log-empty">
          {t("settings.ai.log.empty")}
        </p>
      ) : (
        <div className="overflow-x-auto rounded-[10px] border border-line">
          <table className="w-full border-collapse text-left text-[12.5px]">
            <thead className="bg-surface2 text-[11.5px] font-semibold text-faint">
              <tr>
                <th scope="col" className="px-3 py-2">
                  {t("settings.ai.log.columns.time")}
                </th>
                <th scope="col" className="px-3 py-2">
                  {t("settings.ai.log.columns.meeting")}
                </th>
                <th scope="col" className="px-3 py-2">
                  {t("settings.ai.log.columns.provider")}
                </th>
                <th scope="col" className="px-3 py-2 text-right">
                  {t("settings.ai.log.columns.tokens")}
                </th>
              </tr>
            </thead>
            <tbody>
              {log.map((e, i) => (
                <tr key={`${e.at}-${i}`} className="border-t border-line">
                  <td className="text-mono px-3 py-2.5 text-[11.5px] whitespace-nowrap text-muted">{e.at != null ? `${formatDate(new Date(e.at), lang)} ${formatTime(new Date(e.at), lang)}` : ""}</td>
                  <td className="max-w-[240px] truncate px-3 py-2.5" title={e.meetingTitle}>
                    {e.meetingTitle}
                  </td>
                  <td className="px-3 py-2.5 break-words">
                    {providerName(e.provider)} · {e.model}
                  </td>
                  <td className="text-mono px-3 py-2.5 text-right text-[11.5px] tabular-nums">{nf.format((e.tokensIn ?? 0) + (e.tokensOut ?? 0))}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Card>
  );
}
