// SPDX-License-Identifier: Apache-2.0
// "Pair a phone": the QR code the core rendered (shown as an image; the
// webview never builds or parses it), a countdown, "Show a new code", and on
// success "Paired with <name>" with Unpair. The pairing listener lives only
// while this sheet is open (doc 07 T4): closing or expiry calls syncPairClose.
import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Dialog, Icon } from "@ghi/ui";
import { APP_NAME } from "@ghi/i18n";
import type { DeviceRow } from "../../bindings";
import { ipc } from "../../ipc";
import { DEFAULT_PAIR_TTL_MS, formatCountdown, qrDataUrl } from "./logic";
import { useSyncEvents } from "./use-sync";

type Phase = { kind: "loading" } | { kind: "showing"; src: string } | { kind: "expired" } | { kind: "failed" } | { kind: "paired"; device: DeviceRow };

/** Mounted per open, so every opening starts from a fresh code. */
export function PairSheet({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  return open ? <PairDialog onOpenChange={onOpenChange} /> : null;
}

function PairDialog({ onOpenChange }: { onOpenChange: (open: boolean) => void }) {
  const { t } = useTranslation();
  const [nonce, setNonce] = useState(0); // a new number asks the core for a new code
  const [paired, setPaired] = useState<DeviceRow | null>(null);
  const [now, setNow] = useState(() => Date.now());

  const offer = useQuery({
    queryKey: ["sync-pair-offer", nonce],
    enabled: paired == null,
    gcTime: 0,
    staleTime: Infinity,
    retry: false,
    queryFn: async () => {
      const r = await ipc.commands.syncPairOpen();
      const src = r.status === "ok" ? qrDataUrl(r.data.qrSvg) : null;
      if (r.status !== "ok" || !src) throw new Error("pair_offer");
      return { src, ttl: r.data.expiresMs ?? DEFAULT_PAIR_TTL_MS };
    },
  });
  const deadline = offer.data ? offer.dataUpdatedAt + offer.data.ttl : 0;
  const left = offer.data ? Math.min(offer.data.ttl, Math.max(0, deadline - now)) : 0;
  const expired = !!offer.data && left <= 0;
  const showing = !!offer.data && !expired && paired == null;

  useEffect(() => {
    if (!showing) return;
    const id = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(id);
  }, [showing]);

  // The listener is open only while a code is on screen (T4).
  useEffect(() => {
    if (expired) void ipc.commands.syncPairClose();
  }, [expired]);
  useEffect(() => () => void ipc.commands.syncPairClose(), []);

  useSyncEvents((e) => {
    if (e.type === "paired") setPaired(e.device);
  });

  const regenerate = () => {
    setNow(Date.now());
    setNonce((n) => n + 1);
  };
  const phase: Phase = paired ? { kind: "paired", device: paired } : offer.isError ? { kind: "failed" } : expired ? { kind: "expired" } : offer.data ? { kind: "showing", src: offer.data.src } : { kind: "loading" };

  const unpair = async (device: DeviceRow) => {
    await ipc.commands.syncUnpair(device.gid);
    onOpenChange(false);
  };

  return (
    <Dialog
      open
      onOpenChange={onOpenChange}
      width={420}
      title={phase.kind === "paired" ? t("settings.sync.pairedWith", { device: phase.device.name }) : t("settings.sync.pairTitle")}
      description={phase.kind === "paired" ? t("settings.sync.pairedHint") : t("settings.sync.pairBody", { app: APP_NAME })}
      footer={
        phase.kind === "paired" ? (
          <>
            <Button onClick={() => void unpair(phase.device)}>{t("settings.sync.unpair")}</Button>
            <Button variant="primary" onClick={() => onOpenChange(false)}>
              {t("settings.sync.pairDone")}
            </Button>
          </>
        ) : (
          <Button onClick={() => onOpenChange(false)}>{t("common.cancel")}</Button>
        )
      }
    >
      {phase.kind === "paired" ? (
        <p role="status" className="m-0 flex items-center gap-2 text-body text-accent">
          <Icon name="check_circle" size={20} />
          {t("settings.sync.pairedWith", { device: phase.device.name })}
        </p>
      ) : (
        <div className="flex flex-col items-center gap-3 py-1">
          <div data-testid="pair-qr" className="grid size-[216px] place-items-center rounded-xl border border-line2 bg-white p-2">
            {phase.kind === "showing" ? <img src={phase.src} alt={t("settings.sync.pairQrAlt")} width={200} height={200} /> : <Icon name="schedule" size={28} className="text-muted" />}
          </div>
          {phase.kind === "showing" && (
            <p data-testid="pair-countdown" className="text-small m-0 text-muted">
              {t("settings.sync.pairCountdown", { time: formatCountdown(left) })}
            </p>
          )}
          {phase.kind === "loading" && (
            <p role="status" className="text-small m-0 text-muted">
              {t("settings.sync.pairLoading")}
            </p>
          )}
          {(phase.kind === "expired" || phase.kind === "failed") && (
            <>
              <p role="status" className="text-small m-0 text-warn">
                {phase.kind === "expired" ? t("settings.sync.pairExpired") : t("settings.sync.pairFailed")}
              </p>
              <Button variant="primary" icon="refresh" onClick={regenerate}>
                {t("settings.sync.pairRegenerate")}
              </Button>
            </>
          )}
          {phase.kind === "showing" && (
            <Button size="sm" icon="refresh" onClick={regenerate}>
              {t("settings.sync.pairRegenerate")}
            </Button>
          )}
        </div>
      )}
    </Dialog>
  );
}
