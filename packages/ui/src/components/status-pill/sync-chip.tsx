// SPDX-License-Identifier: Apache-2.0
// The phone's per-meeting chip: where this meeting's processing and sync stand
// (the 16-C `MeetingChip` kinds). Icon + text always. "Failed" becomes a
// button ("Tap to retry") when a retry is possible, plain "Failed" otherwise. Sized in rem so it follows the text scale
// and wraps instead of clipping at 200%.
import { Icon, type IconName } from "../../icons/icon";
import { useMobileT } from "../../utils/mobile-t";
import { cn } from "../../utils/cn";

export type SyncChipKind =
  | { kind: "recorded" }
  | { kind: "processingOnPhone"; percent: number }
  | { kind: "processedOnPhone" }
  | { kind: "waitingForModels" }
  | { kind: "failed" }
  | { kind: "synced" }
  | { kind: "waitingForWifi" }
  | { kind: "finalOnDesktop"; percent: number };

export type SyncChipProps = {
  chip: SyncChipKind;
  /** Name of the paired computer ("finalOnDesktop"); defaults to "My computer". */
  device?: string;
  onRetry?: () => void;
  className?: string;
};

const TONE: Record<SyncChipKind["kind"], { icon: IconName; cls: string }> = {
  recorded: { icon: "mic", cls: "bg-sunk text-muted" },
  processingOnPhone: { icon: "progress_activity", cls: "bg-accent-soft text-accent" },
  processedOnPhone: { icon: "check_circle", cls: "bg-transparent text-muted" },
  waitingForModels: { icon: "hourglass_top", cls: "bg-sunk text-muted" },
  failed: { icon: "error", cls: "bg-sunk text-ink" },
  synced: { icon: "cloud_done", cls: "bg-transparent text-muted" },
  waitingForWifi: { icon: "wifi_off", cls: "bg-sunk text-muted" },
  finalOnDesktop: { icon: "laptop_mac", cls: "bg-accent-soft text-accent" },
};

export function SyncChip({ chip, device, onRetry, className }: SyncChipProps) {
  const t = useMobileT();
  const { icon, cls } = TONE[chip.kind];
  const percent = "percent" in chip ? Math.round(chip.percent) : 0;
  const label = {
    recorded: () => t("mobile.chip.recorded"),
    processingOnPhone: () => t("mobile.chip.processingOnPhone", { percent }),
    processedOnPhone: () => t("mobile.chip.processedOnPhone"),
    waitingForModels: () => t("mobile.chip.waitingForModels"),
    failed: () => t(onRetry ? "mobile.chip.failed" : "mobile.chip.failedNoRetry"),
    synced: () => t("mobile.chip.synced"),
    waitingForWifi: () => t("mobile.chip.waitingForWifi"),
    finalOnDesktop: () => t("mobile.chip.finalOnDesktop", { device: device ?? t("mobile.target.desktop"), percent }),
  }[chip.kind]();
  const base = cn("inline-flex min-h-[1.625rem] items-center gap-1.5 rounded-[1rem] px-2.5 py-0.5 text-start text-ios-caption1 font-semibold", cls, className);
  const content = (
    <>
      <Icon name={icon} size={15} className={cn("size-[1.0625rem] shrink-0", chip.kind === "processingOnPhone" && "animate-spin motion-reduce:animate-none")} />
      {label}
    </>
  );
  if (chip.kind === "failed" && onRetry) {
    // The pill is short; the pseudo-element grows the hit area to 44 pt.
    return (
      <button type="button" onClick={onRetry} data-chip={chip.kind} className={cn(base, "relative before:absolute before:-inset-x-1 before:-inset-y-2.5 before:content-['']")}>
        {content}
      </button>
    );
  }
  return (
    <span data-chip={chip.kind} className={base}>
      {content}
    </span>
  );
}
