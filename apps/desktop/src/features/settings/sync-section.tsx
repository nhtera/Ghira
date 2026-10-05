// SPDX-License-Identifier: Apache-2.0
// Settings → Sync (phase 15): the on/off switch, "Pair a phone" (QR sheet),
// paired devices, what syncs, "Local network only", and what to do when the
// phone can't be found (hotspot / Internet Sharing, then a sealed file to
// export here and import on the other device).
import { useEffect, useRef, useState, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { Button, useToast } from "@ghi/ui";
import { APP_NAME } from "@ghi/i18n";
import { ipc } from "../../ipc";
import { DevicesList } from "../sync/devices-list";
import { errorKey } from "../sync/logic";
import { PairSheet } from "../sync/pair-sheet";
import { TransferSheet, type TransferMode } from "../sync/transfer-sheet";
import { useSyncEvents, useSyncStatus } from "../sync/use-sync";
import { Card, Note, Row, SwitchRow, useFail } from "./parts";

export function SyncSection() {
  const { t } = useTranslation();
  const { show } = useToast();
  const fail = useFail();
  const client = useQueryClient();
  const { data: status, refetch } = useSyncStatus();
  const [pairing, setPairing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [transfer, setTransfer] = useState<TransferMode | null>(null);
  const transferButton = useRef<HTMLButtonElement | null>(null);
  const openTransfer = (mode: TransferMode, e: MouseEvent<HTMLButtonElement>) => {
    transferButton.current = e.currentTarget;
    setTransfer(mode);
  };
  const pairButton = useRef<HTMLButtonElement>(null);

  // The wipe event arrives after the device left the list: remember the names.
  const names = useRef(new Map<string, string>());
  useEffect(() => {
    status?.paired.forEach((d) => names.current.set(d.gid, d.name));
  }, [status]);

  useSyncEvents((e) => {
    if (e.type === "unpaired" && e.byPeer) show({ title: t("settings.sync.unpairedBy", { device: e.name }) });
    if (e.type === "wipeDone") {
      const name = names.current.get(e.gid);
      if (name) show({ tone: "success", title: t("settings.sync.wipeDone", { device: name }) });
    }
  });

  if (!status) return null;
  const enabled = status.enabled;
  const pending = status.pendingOnPhone > 0;
  // Nobody paired yet, or the last session could not reach the phone.
  const noDiscovery = enabled && (status.paired.length === 0 || status.lastErrorCode === "unreachable");

  const toggle = async (on: boolean) => {
    const r = await ipc.commands.syncSetEnabled(on);
    if (r.status === "error") return fail(r.error);
    client.setQueryData(["sync-status"], r.data);
  };
  const syncNow = async () => {
    setBusy(true);
    const r = await ipc.commands.syncNow();
    setBusy(false);
    if (r.status === "error") fail(r.error);
    void refetch();
  };

  return (
    <div className="flex flex-col">
      <SwitchRow label={t("settings.sync.toggle")} hint={t("settings.sync.toggleHint")} checked={enabled} onChange={(v) => void toggle(v)} testId="sync-toggle" />
      {enabled && (
        <>
          <Row label={t("settings.sync.pairTitle")} hint={t("settings.sync.pairBody", { app: APP_NAME })}>
            <Button ref={pairButton} variant="primary" icon="mobile" onClick={() => setPairing(true)}>
              {t("settings.sync.pairTitle")}
            </Button>
          </Row>
          <Card title={t("settings.sync.pairedDevices")}>
            {status.paired.length === 0 ? (
              <p className="text-small m-0 text-muted">{t("settings.sync.noDevices")}</p>
            ) : (
              <DevicesList devices={status.paired} offline={status.lastErrorCode != null} onChanged={() => void refetch()} />
            )}
            {pending && <Note icon="phone_paused">{t("settings.sync.openOnPhone", { app: APP_NAME })}</Note>}
            {status.lastErrorCode && (
              <p role="alert" data-testid="sync-error" className="text-small m-0 text-warn">
                {t(`settings.sync.error.${errorKey(status.lastErrorCode)}`, { app: APP_NAME })}
              </p>
            )}
            {status.paired.length > 0 && (
              <div>
                <Button size="sm" icon="sync" disabled={busy} onClick={() => void syncNow()}>
                  {t("settings.sync.syncNow")}
                </Button>
              </div>
            )}
          </Card>
          <Card title={t("settings.sync.whatSyncs")}>
            <ul className="m-0 flex list-disc flex-col gap-1 ps-5 text-[13.5px]">
              <li>{t("settings.sync.phoneRecordings")}</li>
              <li>{t("settings.sync.notes")}</li>
            </ul>
            <Note icon="info">{t("settings.sync.voiceStay")}</Note>
          </Card>
          {noDiscovery && (
            <Card title={t("settings.sync.hotspotTitle")} hint={t("settings.sync.hotspotBody")}>
              <div className="flex flex-wrap items-center gap-3">
                <Button icon="ios_share" onClick={(e) => openTransfer("export", e)} aria-describedby="export-instead-hint">
                  {t("settings.sync.exportInstead")}
                </Button>
                <span id="export-instead-hint" className="text-small text-muted">
                  {t("settings.sync.exportInsteadHint")}
                </span>
              </div>
            </Card>
          )}
        </>
      )}
      <Card className="mt-2">
        <Row label={t("settings.sync.importFromDevice")} hint={t("settings.sync.importFromDeviceHint", { app: APP_NAME })}>
          <Button icon="upload_file" onClick={(e) => openTransfer("import", e)}>
            {t("settings.sync.importFromDevice")}
          </Button>
        </Row>
      </Card>
      <div className="mt-5">
        <Note icon="wifi">{t("settings.sync.localOnly")}</Note>
      </div>
      <TransferSheet
        mode={transfer}
        onOpenChange={(o) => {
          if (o) return;
          setTransfer(null);
          // The sheet is unmounted with its dialog: give focus back to the button.
          requestAnimationFrame(() => transferButton.current?.focus());
        }}
      />
      <PairSheet
        open={pairing}
        onOpenChange={(o) => {
          setPairing(o);
          // The sheet is unmounted with its dialog: give focus back to the button.
          if (!o) requestAnimationFrame(() => pairButton.current?.focus());
        }}
      />
    </div>
  );
}
