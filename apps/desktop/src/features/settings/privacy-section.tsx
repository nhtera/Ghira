// SPDX-License-Identifier: Apache-2.0
// Settings → Privacy: encryption, audio retention, strict offline, export
// everything, delete everything.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, InlineConfirm, Segmented, cn, useToast, usePlatform } from "@ghi/ui";
import { type Locale } from "@ghi/i18n";
import { ipc } from "../../ipc";
import { MIN_PASSWORD, RETENTION_DAYS, deleteWord, deleteWordMatches, passwordIssue, passwordStrength, retentionDeletes } from "./logic";
import { AppLockCard } from "./app-lock-card";
import { MyVoiceCard } from "./my-voice-card";
import { DeleteEverywhereAsk, DeleteWaiting, useDeleteEverywhereStatus } from "../sync/delete-everywhere";
import { useSyncStatus } from "../sync/use-sync";
import { Card, Note, Row, Switch, SwitchRow, inputCls, useFail, useSettings } from "./parts";

export function PrivacySection() {
  const { t } = useTranslation();
  const { settings, patch } = useSettings();
  const { data: sync } = useSyncStatus();
  return (
    <div className="flex flex-col">
      <Card>
        <Row label={t("settings.privacy.encrypt")} hint={t("settings.privacy.encryptHint")}>
          <Switch checked onChange={() => {}} label={t("settings.privacy.encrypt")} disabled />
        </Row>
      </Card>
      <AppLockCard />
      {settings && <Retention days={settings.audioRetentionDays} onApply={(d) => patch({ audioRetentionDays: d })} />}
      <MyVoiceCard />
      <Card>
        <Row label={t("settings.privacy.learnVoices")} hint={t("settings.privacy.thirdPartyOff")}>
          <Switch checked={false} onChange={() => {}} label={t("settings.privacy.learnVoices")} disabled />
        </Row>
        {settings && <SwitchRow label={t("settings.privacy.strictOffline")} hint={t("settings.privacy.strictOfflineHint")} checked={settings.strictOffline} onChange={(v) => void patch({ strictOffline: v })} testId="strict-offline" />}
        {sync?.enabled && (
          <div data-testid="strict-offline-local-note">
            <Note icon="wifi">{t("settings.sync.localOnly")}</Note>
          </div>
        )}
      </Card>
      <ExportCard />
      <DeleteCard />
    </div>
  );
}

function Retention({ days, onApply }: { days: number; onApply: (d: number) => Promise<unknown> }) {
  const { t } = useTranslation();
  const [asked, setAsked] = useState<number | null>(null);
  const label = (d: number) => (d === 0 ? t("settings.privacy.keepOptions.forever") : t("settings.privacy.days", { count: d }));
  return (
    <>
      <Row label={t("settings.privacy.keepAudio")} hint={t("settings.privacy.keepAudioHint")}>
        <Segmented<string>
          label={t("settings.privacy.keepAudio")}
          value={String(days)}
          onChange={(v) => {
            const next = Number(v);
            if (retentionDeletes(days, next)) setAsked(next);
            else {
              setAsked(null);
              void onApply(next);
            }
          }}
          options={[...RETENTION_DAYS.filter((d) => d), 0].map((d) => ({
            value: String(d),
            label: label(d),
          }))}
        />
      </Row>
      {asked != null && (
        <div role="region" aria-label={t("settings.privacy.keepAudio")} className="py-3">
          <RetentionConfirm
            days={asked}
            onConfirm={() => {
              void onApply(asked);
              setAsked(null);
            }}
            onCancel={() => setAsked(null)}
          />
        </div>
      )}
    </>
  );
}

function RetentionConfirm({ days, onConfirm, onCancel }: { days: number; onConfirm: () => void; onCancel: () => void }) {
  const { t } = useTranslation();
  return <InlineConfirm icon="warning" question={t("settings.privacy.retentionConfirm", { count: days })} confirmLabel={t("settings.privacy.retentionApply")} onConfirm={onConfirm} onCancel={onCancel} />;
}

const STRENGTH_CLS = ["bg-rec", "bg-warn", "bg-accent", "bg-accent"] as const;

function ExportCard() {
  const { t } = useTranslation();
  const { show } = useToast();
  const context = usePlatform();
  const fail = useFail();
  const [pw, setPw] = useState("");
  const [again, setAgain] = useState("");
  const [busy, setBusy] = useState(false);
  const issue = passwordIssue(pw, again);
  const strength = passwordStrength(pw);
  const run = async () => {
    setBusy(true);
    const r = await ipc.commands.exportEverything(pw);
    setBusy(false);
    if (r.status === "error") return fail(r.error);
    setPw("");
    setAgain("");
    if (r.data == null) return; // the save dialog was cancelled
    show({
      tone: "success",
      title: t("settings.privacy.exported", { name: r.data }),
      action: {
        label: t("export.reveal", { context }),
        altText: t("export.reveal", { context }),
        onAction: () => void ipc.commands.revealLastExport(),
      },
    });
  };
  return (
    <div className="border-b border-line py-3.5">
      <div className="text-[14px] font-medium">{t("settings.privacy.exportAll")}</div>
      <div className="text-[12.5px] leading-normal text-muted">{t("settings.privacy.exportAllHint")}</div>
      <form
        className="mt-3 flex max-w-sm flex-col gap-2.5"
        onSubmit={(e) => {
          e.preventDefault();
          if (!issue) void run();
        }}
      >
        <label className="flex flex-col gap-1 text-small font-semibold">
          {t("settings.privacy.password")}
          <input type="password" autoComplete="new-password" className={cn(inputCls, "font-normal")} value={pw} onChange={(e) => setPw(e.target.value)} />
        </label>
        <label className="flex flex-col gap-1 text-small font-semibold">
          {t("settings.privacy.passwordAgain")}
          <input type="password" autoComplete="new-password" className={cn(inputCls, "font-normal")} value={again} onChange={(e) => setAgain(e.target.value)} />
        </label>
        {pw && (
          <div className="flex items-center gap-2" data-testid="password-strength">
            <i className="flex h-1.5 flex-1 overflow-hidden rounded-[3px] bg-sunk">
              <i className={cn("block h-full", STRENGTH_CLS[strength])} style={{ width: `${((strength + 1) / 4) * 100}%` }} />
            </i>
            <span className="text-small text-muted">{t(`settings.privacy.strength.${strength}`)}</span>
          </div>
        )}
        <span role="status" className="text-small min-h-4 text-warn">
          {pw && issue && (issue === "short" ? t("settings.privacy.passwordShort", { min: MIN_PASSWORD }) : again ? t("settings.privacy.passwordMismatch") : "")}
        </span>
        <Note icon="warning">{t("settings.privacy.passwordWarning")}</Note>
        <div>
          <Button type="submit" variant="primary" icon="ios_share" disabled={!!issue || busy}>
            {t("settings.privacy.exportButton")}
          </Button>
        </div>
      </form>
    </div>
  );
}

function DeleteCard() {
  const { t, i18n } = useTranslation();
  const fail = useFail();
  const word = deleteWord(i18n.language as Locale);
  const [typed, setTyped] = useState("");
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [asking, setAsking] = useState(false);
  const { data: sync } = useSyncStatus();
  const paired = sync?.enabled ? sync.paired.map((d) => d.name) : [];
  const waiting = useDeleteEverywhereStatus(busy && paired.length > 0);
  // One call only: a second one would hit the store mid-wipe. On success the
  // app restarts, so the busy state stays; on failure it is released.
  const run = async () => {
    if (busy) return;
    setAsking(false);
    setBusy(true);
    const r = await ipc.commands.deleteAllData();
    if (r.status === "error") {
      fail(r.error);
      setBusy(false);
    }
  };
  // "Delete here only": sync goes off first, so nobody is waited for.
  const hereOnly = async () => {
    const off = await ipc.commands.syncSetEnabled(false);
    if (off.status === "error") return fail(off.error);
    if (!busy) void run();
  };
  return (
    <Card title={t("settings.privacy.dangerTitle")} hint={t("settings.privacy.dangerBody")}>
      {!open ? (
        <Button className="border-rec text-rec" onClick={() => setOpen(true)}>
          {t("settings.privacy.deleteAll")}
        </Button>
      ) : (
        <form
          className="flex max-w-sm flex-col gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (!deleteWordMatches(typed, word) || busy) return;
            if (paired.length > 0) setAsking(true);
            else void run();
          }}
        >
          <label className="flex flex-col gap-1 text-small font-semibold">
            {t("settings.privacy.typeToConfirm", { word })}
            <input autoFocus autoComplete="off" className={cn(inputCls, "font-normal")} value={typed} disabled={busy} onChange={(e) => setTyped(e.target.value)} />
          </label>
          {asking && <DeleteEverywhereAsk devices={paired} onEverywhere={() => void run()} onHereOnly={() => void hereOnly()} onCancel={() => setAsking(false)} />}
          {busy && waiting?.state === "waiting" && <DeleteWaiting names={waiting.waitingFor} onHereOnly={() => void hereOnly()} />}
          <div className="flex gap-2">
            <Button type="submit" variant="danger" disabled={busy || asking || !deleteWordMatches(typed, word)}>
              {t("settings.privacy.deleteEverything")}
            </Button>
            <Button
              type="button"
              disabled={busy}
              onClick={() => {
                setOpen(false);
                setAsking(false);
                setTyped("");
              }}
            >
              {t("common.cancel")}
            </Button>
          </div>
        </form>
      )}
    </Card>
  );
}
