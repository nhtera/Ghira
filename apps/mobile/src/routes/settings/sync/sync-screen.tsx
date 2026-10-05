// SPDX-License-Identifier: Apache-2.0
// Settings → Sync with computer (phase 15): pair by scanning the code the
// computer shows, see who is paired and when it last synced, unpair (data
// stays) or unpair and have the computer delete what it got from this phone.
// Sync is local network only; when the Wi-Fi hides devices from each other,
// Personal Hotspot is the way, then the file export.
import { ListRow, ListSection, Sheet } from "@ghi/ui";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { SyncEvent } from "../../../bindings";
import { ipc } from "../../../ipc";
import { unwrap, useAction } from "../../../features/settings/api";
import { Btn } from "../../../features/settings/controls";
import { Page } from "../../../features/settings/page";
import { relativeTime } from "../../../features/sync/format";
import { HotspotHelp, PairScanPanel } from "../../../features/sync/pair-scan";
import { syncErrorKey } from "../../../features/sync/scan-error";
import { usePairScan } from "../../../features/sync/use-pair-scan";
import { useSync, useSyncEvents } from "../../../features/sync/use-sync";

type Confirm = "unpair" | "wipe";

export function SyncScreen() {
  const { t, i18n } = useTranslation();
  const sync = useSync();
  const scan = usePairScan(false);
  const resetScan = scan.reset;
  const action = useAction();
  const [confirm, setConfirm] = useState<Confirm | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const wipingName = useRef<string | null>(null);
  const { status, device, wiping } = sync;
  // The name outlives the device row: the wipe's end arrives after the row is gone.
  useEffect(() => {
    if (wiping) wipingName.current = wiping.name;
  }, [wiping]);
  const [now] = useState(() => Date.now());

  useSyncEvents(
    useCallback(
      (e: SyncEvent) => {
        if (e.type === "unpaired") resetScan();
        if (e.type === "unpaired" && e.byPeer) setNotice(t("mobile.sync.unpairedBy", { device: e.name }));
        if (e.type === "wipeDone") setNotice(t("mobile.sync.screen.wipeDone", { device: wipingName.current ?? t("mobile.target.desktop") }));
        if (e.type === "paired") setNotice(null);
      },
      [t, resetScan],
    ),
  );

  const close = () => {
    setConfirm(null);
    action.clear();
  };
  const unpair = (wipe: boolean) =>
    void action.run(async () => {
      if (!device) return;
      unwrap(wipe ? await ipc.commands.syncUnpairAndWipe(device.gid) : await ipc.commands.syncUnpair(device.gid));
      setConfirm(null);
      scan.reset();
      sync.reload();
    });
  const syncNow = () =>
    void action.run(async () => {
      unwrap(await ipc.commands.syncNow());
      sync.reload();
    });

  const named = device ?? wiping;
  const lastSynced = named ? (named.lastSeenMs === null ? t("mobile.sync.screen.neverSynced") : t("mobile.sync.screen.lastSynced", { when: relativeTime(named.lastSeenMs, now, i18n.language) })) : null;

  const localOnly = (
    <ListSection>
      <ListRow title={t("mobile.sync.screen.localOnly")} subtitle={t("mobile.sync.screen.localOnlyBody")} icon="wifi" />
    </ListSection>
  );
  const note = (text: ReactNode) => (
    <p role="status" className="text-ios-footnote mx-4 my-2 text-muted">
      {text}
    </p>
  );

  return (
    <Page title={t("mobile.sync.row")} back="settings" error={sync.loadError} onRetry={sync.reload}>
      {status && (
        <>
          {notice && note(notice)}
          {action.error && (
            <p role="alert" className="text-ios-footnote mx-4 my-2 text-rec-ink">
              {t(`mobile.sync.error.${syncErrorKey(action.error)}`)}
            </p>
          )}

          {device && (
            <>
              <ListSection header={t("mobile.sync.screen.pairedHeader")}>
                {/* The computer's name is untrusted text: a text node only. */}
                <ListRow title={device.name} subtitle={lastSynced ?? undefined} icon="laptop_mac" />
                {status.pendingOnPhone > 0 && <ListRow title={t("mobile.sync.screen.pending", { count: status.pendingOnPhone })} icon="sync" />}
              </ListSection>
              {status.lastErrorCode && (
                <p role="alert" className="text-ios-footnote mx-4 my-2 text-rec-ink">
                  {t(`mobile.sync.error.${syncErrorKey(status.lastErrorCode)}`)}
                </p>
              )}
              <div className="mx-4 mb-1 flex flex-col items-start gap-3">
                <Btn tone="primary" disabled={action.busy} onClick={syncNow}>
                  {action.busy ? t("mobile.sync.screen.syncing") : t("mobile.sync.screen.syncNow")}
                </Btn>
              </div>
              {localOnly}
              <div className="mx-4 flex flex-col items-start gap-3">
                <Btn onClick={() => setConfirm("unpair")}>{t("mobile.sync.unpair")}</Btn>
                <Btn tone="danger" onClick={() => setConfirm("wipe")}>
                  {t("mobile.sync.screen.unpairAndWipe", { device: device.name })}
                </Btn>
              </div>
            </>
          )}

          {!device && wiping && (
            <>
              <ListSection header={t("mobile.sync.screen.pairedHeader")}>
                <ListRow title={wiping.name} icon="laptop_mac" />
              </ListSection>
              <p role="status" data-testid="wipe-pending" className="text-ios-subhead mx-4 my-2 text-muted">
                {t("mobile.sync.screen.wipePending", { device: wiping.name })}
              </p>
              {localOnly}
            </>
          )}

          {!device && !wiping && (
            <>
              <p className="text-ios-subhead mx-4 mt-2 mb-4 text-muted">{t("mobile.sync.screen.off")}</p>
              <div className="mx-4">
                <PairScanPanel scan={scan} />
              </div>
              {localOnly}
              {scan.phase !== "failed" && <HotspotHelp className="mx-4 mb-4 px-1" />}
            </>
          )}
        </>
      )}

      <Sheet
        open={confirm !== null && device !== null}
        onOpenChange={(o) => !o && close()}
        title={confirm === "wipe" ? t("mobile.sync.screen.unpairAndWipeTitle", { device: device?.name ?? "" }) : t("mobile.sync.unpairTitle", { device: device?.name ?? "" })}
        description={confirm === "wipe" ? t("mobile.sync.screen.unpairAndWipeBody", { device: device?.name ?? "" }) : t("mobile.sync.unpairBody", { device: device?.name ?? "" })}
        closeLabel={t("mobile.sheet.close")}
        handleLabel={t("mobile.sheet.handle")}
        dismissible={!action.busy}
        footer={
          <div className="flex flex-col gap-2">
            <Btn tone={confirm === "wipe" ? "danger" : "primary"} disabled={action.busy} onClick={() => unpair(confirm === "wipe")}>
              {confirm === "wipe" ? t("mobile.sync.screen.unpairAndWipe", { device: device?.name ?? "" }) : t("mobile.sync.unpair")}
            </Btn>
            <Btn disabled={action.busy} onClick={close}>
              {t("mobile.common.cancel")}
            </Btn>
          </div>
        }
      />
    </Page>
  );
}
