// SPDX-License-Identifier: Apache-2.0
// The pairing panel the onboarding step and Settings share: the scan
// viewfinder, the paired card, and a failure with a way out (retry, Settings,
// Personal Hotspot guidance). The camera itself is native; here the web draws
// the state around it.
import { Banner, Icon, PhoneButton } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import { ipc } from "../../ipc";
import { needsSettings, scanFailureKey, wantsHotspot } from "./scan-error";
import type { PairScan } from "./use-pair-scan";

/** When the computer cannot be found: Personal Hotspot first, then the file export. */
export function HotspotHelp({ className }: { className?: string }) {
  const { t } = useTranslation();
  return (
    <div data-testid="hotspot-help" className={className}>
      <p className="text-ios-subhead m-0 flex items-start gap-2.5 font-semibold">
        <Icon name="wifi" size={20} className="mt-0.5 size-5 shrink-0 text-accent" />
        {t("mobile.sync.hotspot.title")}
      </p>
      <p className="text-ios-footnote m-0 mt-1 text-muted">{t("mobile.sync.hotspot.body")}</p>
      <p className="text-ios-footnote m-0 mt-1 text-muted">{t("mobile.sync.hotspot.export")}</p>
    </div>
  );
}

export function PairedCard({ device }: { device: string }) {
  const { t } = useTranslation();
  return (
    <div role="status" data-testid="paired-card" className="flex items-start gap-3 rounded-(--ios-radius-group) bg-accent-soft p-4">
      <Icon name="laptop_mac" size={28} className="size-7 shrink-0 text-accent" />
      <div className="min-w-0">
        {/* The computer's name is untrusted text: a text node only. */}
        <p className="text-ios-callout m-0 font-semibold break-words text-accent">{t("mobile.pair.paired", { device })}</p>
        <p className="text-ios-subhead m-0 text-ink">{t("mobile.pair.syncNote")}</p>
      </div>
    </div>
  );
}

export function PairScanPanel({ scan }: { scan: PairScan }) {
  const { t } = useTranslation();
  const { phase, failure, device } = scan;

  if (phase === "paired" && device) return <PairedCard device={device.name} />;

  const scanning = phase === "scanning";
  return (
    <div className="flex flex-col gap-4">
      <div
        data-testid="pair-viewfinder"
        data-phase={phase}
        role="img"
        aria-label={scanning ? t("mobile.sync.scan.viewfinder") : t("mobile.pair.title")}
        className="relative grid h-[min(16.25rem,40vh)] place-items-center rounded-[20px] bg-[repeating-linear-gradient(135deg,#1E2724_0_12px,#252F2B_12px_24px)]"
      >
        <span aria-hidden="true" className="aspect-square h-[70%] max-w-[80%] rounded-[18px] border-[3px] border-white opacity-90" />
        <span aria-hidden="true" className="text-ios-caption1 absolute bottom-3.5 rounded-md bg-black/50 px-2 py-1 font-mono text-white">
          {scanning ? t("mobile.pair.scanning") : t("mobile.sync.pairWithDesktop")}
        </span>
      </div>
      {phase === "failed" && failure && (
        <>
          <Banner
            variant="warning"
            title={t(`mobile.sync.${scanFailureKey(failure)}`)}
            action={needsSettings(failure) ? { label: t("mobile.common.openSettings"), onPress: () => void ipc.commands.openAppSettings() } : undefined}
          />
          {wantsHotspot(failure) && <HotspotHelp />}
        </>
      )}
      {(phase === "failed" || phase === "idle") && (
        <PhoneButton variant={phase === "failed" ? "secondary" : "primary"} size="compact" onClick={() => void scan.start()}>
          {phase === "failed" ? t("mobile.sync.scan.tryAgain") : t("mobile.sync.scan.start")}
        </PhoneButton>
      )}
    </div>
  );
}
