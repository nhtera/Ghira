// SPDX-License-Identifier: Apache-2.0
// Settings → Recording: which apps prompt to record, echo cancellation and
// per-app capture, the microphone permission, the consent message.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { Button, Icon, Segmented, useToast, usePlatform } from "@ghi/ui";
import { APP_NAME } from "@ghi/i18n";
import type { LiveMode } from "../../bindings";
import { ipc } from "../../ipc";
import { CalendarCard } from "../calendar/calendar-card";
import { Card, Note, Row, SwitchRow, bigSegCls, useFail, useSettings } from "./parts";

export function RecordingSection() {
  const { t } = useTranslation();
  const context = usePlatform();
  const { settings, patch } = useSettings();
  const { data: mic } = useQuery({
    queryKey: ["mic-permission"],
    queryFn: () => ipc.commands.micPermission(),
  });
  return (
    <div className="flex flex-col">
      <Card>
        {settings && (
          <>
            <DetectApps apps={settings.detectMeetings ? settings.detectApps : []} onChange={(detectApps) => void patch({ detectApps, detectMeetings: detectApps.length > 0 })} />
            <SwitchRow label={t("settings.recording.echoCancel")} hint={t("settings.recording.echoCancelHint")} checked={settings.echoCancellation} onChange={(v) => void patch({ echoCancellation: v })} />
            <SwitchRow label={t("settings.recording.appAudioOnly")} hint={t("settings.recording.appAudioOnlyHint")} checked={settings.appAudioOnly} onChange={(v) => void patch({ appAudioOnly: v })} />
          </>
        )}
        <Row label={t("settings.recording.micTitle")} hint={mic && t(`settings.recording.mic.${mic}`)}>
          {mic && mic !== "unsupported" && mic !== "granted" && <Button size="lg" onClick={() => void ipc.commands.openPrivacySettings("microphone")}>{t("settings.recording.openPrivacy", { context })}</Button>}
          {mic === "granted" && <Icon name="check_circle" size={20} className="text-accent" />}
        </Row>
      </Card>
      <CalendarCard />
      {settings && <LiveModeCard mode={settings.liveMode} onChange={(liveMode) => void patch({ liveMode })} />}
      <Card title={t("settings.recording.consentMessage")} hint={t("settings.recording.consentMessageHint")}>
        {settings && (
          <>
            <ConsentEditor lang="en" value={settings.consentMessageEn} onSave={(v) => patch({ consentMessageEn: v })} />
            <ConsentEditor lang="vi" value={settings.consentMessageVi} onSave={(v) => patch({ consentMessageVi: v })} />
          </>
        )}
      </Card>
    </div>
  );
}

/** The meeting apps that may prompt "record this call?" (the master switch is `detectMeetings`). */
const DETECT_APPS = ["zoom", "teams", "meet", "slack", "zalo", "webex", "facetime"] as const;

function DetectApps({ apps, onChange }: { apps: string[]; onChange: (apps: string[]) => void }) {
  const { t } = useTranslation();
  return (
    <div data-row className="flex flex-col gap-2.5 border-b border-line py-3.5">
      <div>
        <div className="text-[14px] font-semibold">{t("settings.recording.detectApps")}</div>
        <div className="text-[12.5px] leading-normal text-muted">{t("settings.recording.neverAuto", { app: APP_NAME })}</div>
      </div>
      <ul className="m-0 flex list-none flex-wrap gap-2 p-0">
        {DETECT_APPS.map((id) => {
          const on = apps.includes(id);
          return (
            <li key={id}>
              <button
                type="button"
                aria-pressed={on}
                onClick={() => onChange(on ? apps.filter((a) => a !== id) : [...apps, id])}
                className={`inline-flex h-8 items-center gap-1 rounded-full border-[1.5px] px-3 text-[13px] font-medium ${on ? "border-accent bg-accent-soft text-accent" : "border-ctl bg-surface text-muted hover:bg-surface2"}`}
              >
                <Icon name={on ? "check" : "add"} size={16} />
                {t(`settings.recording.app.${id}`)}
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

export function LiveModeCard({ mode, onChange }: { mode: LiveMode; onChange: (m: LiveMode) => void }) {
  const { t } = useTranslation();
  const context = usePlatform();
  const { data: tier } = useQuery({
    queryKey: ["models-tier"],
    queryFn: async () => {
      const r = await ipc.commands.modelsStatus();
      return r.status === "ok" ? r.data.tier : null;
    },
  });
  // 8 GB computers always run Fast: shown, but not changeable.
  const locked = tier === "light";
  const shown: LiveMode = locked ? "fast" : mode;
  const label = t("settings.recording.liveMode.title");
  return (
    <Card title={label}>
      <fieldset disabled={locked} className="m-0 min-w-0 border-0 p-0">
        <Segmented<LiveMode>
          label={label}
          className={bigSegCls}
          value={shown}
          onChange={onChange}
          options={[
            { value: "auto", label: t("settings.recording.liveMode.auto") },
            { value: "fast", label: t("settings.recording.liveMode.fast") },
            {
              value: "accurate",
              label: t("settings.recording.liveMode.accurate"),
            },
          ]}
        />
      </fieldset>
      <Note>{locked ? t("settings.recording.liveMode.lightTier", { context }) : t(`settings.recording.liveMode.${shown}Desc`)}</Note>
      {!locked && <Note icon="schedule">{t("settings.recording.liveMode.applies")}</Note>}
    </Card>
  );
}

function ConsentEditor({ lang, value, onSave }: { lang: "en" | "vi"; value: string; onSave: (v: string) => Promise<unknown> }) {
  const { t } = useTranslation();
  const { show } = useToast();
  const fail = useFail();
  // null: not edited, show the saved value (so a reset or reload shows through).
  const [draft, setDraft] = useState<string | null>(null);
  const text = draft ?? value;
  const label = t(lang === "en" ? "settings.recording.consentEn" : "settings.recording.consentVi");
  const builtin = t("live.consent.message", { app: APP_NAME, lng: lang });
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text.trim() || builtin);
      show({ tone: "success", title: t("live.consent.copied") });
    } catch (e) {
      fail(String(e));
    }
  };
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-start gap-3 rounded-[10px] bg-surface2 px-3.5 py-3">
        <span className="w-[22px] pt-[3px] text-[11px] font-bold text-faint" aria-hidden>
          {lang.toUpperCase()}
        </span>
        <textarea
          id={`consent-${lang}`}
          aria-label={label}
          lang={lang}
          rows={2}
          className="min-w-0 flex-1 resize-none border-0 bg-transparent p-0 font-[family-name:var(--font-serif)] text-[15px] leading-[1.55] text-ink focus-visible:outline-2 focus-visible:outline-accent"
          value={text}
          placeholder={builtin}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => {
            if (text.trim() !== value) void onSave(text.trim());
            setDraft(null);
          }}
        />
        <Button size="lg" onClick={() => void copy()} aria-label={t("settings.recording.copyFor", { label })}>
          {t("common.copy")}
        </Button>
      </div>
      <div>
        <Button
          size="sm"
          variant="ghost"
          disabled={!value && !text}
          onClick={() => {
            setDraft(null);
            void onSave("");
          }}
        >
          {t("settings.recording.consentReset")}
        </Button>
      </div>
    </div>
  );
}
