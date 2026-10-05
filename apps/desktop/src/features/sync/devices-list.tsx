// SPDX-License-Identifier: Apache-2.0
// Paired devices: name, platform, "Synced 4 min ago" / "Last seen", Unpair, and
// "Unpair and wipe" with the honest copy of doc 07 T7 (what is already on the
// phone stays readable; the wipe happens when it next connects).
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Dialog, Icon } from "@ghi/ui";
import { APP_NAME, type Locale } from "@ghi/i18n";
import type { DeviceRow } from "../../bindings";
import { ipc } from "../../ipc";
import { useFail } from "../settings/parts";
import { whenAgo } from "./logic";

type Asking = { device: DeviceRow; wipe: boolean };

export function DevicesList({ devices, offline, onChanged }: { devices: DeviceRow[]; /** The last session failed: "Last seen" instead of "Synced". */ offline: boolean; onChanged: () => void }) {
  const { t, i18n } = useTranslation();
  const fail = useFail();
  const [asking, setAsking] = useState<Asking | null>(null);
  const [busy, setBusy] = useState(false);
  // "4 min ago" moves on without a new event.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(id);
  }, []);

  const confirm = async () => {
    if (!asking || busy) return;
    setBusy(true);
    const r = asking.wipe ? await ipc.commands.syncUnpairAndWipe(asking.device.gid) : await ipc.commands.syncUnpair(asking.device.gid);
    setBusy(false);
    setAsking(null);
    if (r.status === "error") fail(r.error);
    onChanged();
  };

  const status = (d: DeviceRow) => {
    if (d.state === "wipePending") return t("settings.sync.wipePending", { app: APP_NAME, device: d.name });
    if (d.lastSeenMs == null) return t("settings.sync.neverSynced");
    const when = whenAgo(d.lastSeenMs, now, i18n.language as Locale, t("settings.sync.justNow"));
    return t(offline ? "settings.sync.lastSeen" : "settings.sync.syncedAt", { when });
  };

  return (
    <>
      <ul className="m-0 flex list-none flex-col p-0" aria-label={t("settings.sync.pairedDevices")}>
        {devices.map((d) => (
          <li key={d.gid} data-testid={`device-${d.gid}`} className="flex flex-wrap items-center gap-3 border-b border-line py-3.5">
            <Icon name={d.platform === "ios" ? "mobile" : d.platform === "mac" ? "laptop_mac" : "desktop_windows"} size={22} className="text-muted" />
            <div className="min-w-0 flex-1">
              <div className="text-[14px] font-medium">
                {d.name}
                <span className="ms-2 text-small font-normal text-muted">{t(`settings.sync.platform.${d.platform}`)}</span>
              </div>
              <div className="text-[12.5px] leading-normal text-muted">{status(d)}</div>
            </div>
            {d.state === "paired" && (
              <>
                <Button size="sm" onClick={() => setAsking({ device: d, wipe: false })}>
                  {t("settings.sync.unpair")}
                </Button>
                <Button size="sm" className="border-rec text-rec" onClick={() => setAsking({ device: d, wipe: true })}>
                  {t("settings.sync.unpairAndWipe")}
                </Button>
              </>
            )}
          </li>
        ))}
      </ul>
      <Dialog
        open={asking != null}
        onOpenChange={(o) => !o && !busy && setAsking(null)}
        width={460}
        title={asking ? t(asking.wipe ? "settings.sync.wipeTitle" : "settings.sync.unpairTitle", { device: asking.device.name }) : ""}
        description={asking ? (asking.wipe ? t("settings.sync.wipeBody", { app: APP_NAME, device: asking.device.name }) : t("settings.sync.unpairBody", { device: asking.device.name })) : undefined}
        footer={
          <>
            <Button onClick={() => setAsking(null)} disabled={busy}>
              {t("common.cancel")}
            </Button>
            <Button variant="danger" onClick={() => void confirm()} disabled={busy}>
              {asking?.wipe ? t("settings.sync.unpairAndWipe") : t("settings.sync.unpair")}
            </Button>
          </>
        }
      >
        {asking?.wipe && (
          <p role="note" className="text-small m-0 flex items-start gap-2 text-warn">
            <Icon name="warning" size={16} className="mt-px shrink-0" />
            <span>{t("settings.sync.wipeHonest", { app: APP_NAME })}</span>
          </p>
        )}
      </Dialog>
    </>
  );
}
