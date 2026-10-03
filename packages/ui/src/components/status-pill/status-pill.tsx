// SPDX-License-Identifier: Apache-2.0
// Meeting status pill (library rows, detail header). Icon + text always; the
// tint only reinforces. "Failed" becomes a button when a retry is possible.
import { useTranslation } from "react-i18next";
import { Icon, type IconName } from "../../icons/icon";
import { usePlatformContext } from "../../platform/platform";
import { cn } from "../../utils/cn";

export type StatusKind =
  | "ready"
  | "processing"
  | "finalPass"
  | "needsNames"
  | "cloudEnhanced"
  | "failed"
  | "waitingSync"
  | "waitingModels"
  | "recording";

export type StatusPillProps = {
  status: StatusKind;
  /** 0..100, shown for "processing" and "finalPass" when known. */
  percent?: number;
  /** Makes "failed" an actionable "Failed · Retry" button. */
  onRetry?: () => void;
  className?: string;
};

const TONE: Record<StatusKind, { icon: IconName; cls: string }> = {
  ready: { icon: "check_circle", cls: "bg-transparent text-muted" },
  processing: { icon: "progress_activity", cls: "bg-accent-soft text-accent" },
  finalPass: { icon: "sync", cls: "bg-accent-soft text-accent" },
  needsNames: { icon: "record_voice_over", cls: "bg-warn-soft text-warn" },
  cloudEnhanced: { icon: "cloud", cls: "bg-warn-soft text-warn" },
  failed: { icon: "error", cls: "bg-sunk text-ink" },
  waitingSync: { icon: "sync", cls: "bg-sunk text-muted" },
  waitingModels: { icon: "hourglass_top", cls: "bg-sunk text-muted" },
  recording: { icon: "radio_button_checked", cls: "bg-rec-soft text-rec-ink" },
};

export function StatusPill({ status, percent, onRetry, className }: StatusPillProps) {
  const { t } = useTranslation();
  const ctx = usePlatformContext();
  const { icon, cls } = TONE[status];
  const label = {
    ready: () => t("library.status.ready"),
    processing: () =>
      percent === undefined ? t("library.status.processing") : t("library.status.processingPercent", { percent: Math.round(percent) }),
    finalPass: () =>
      percent === undefined
        ? t("library.status.finalPassLocal", { context: ctx("library.status.finalPassLocal") })
        : t("library.status.finalPassLocalPercent", { context: ctx("library.status.finalPassLocalPercent"), percent: Math.round(percent) }),
    needsNames: () => t("library.status.needsSpeakerNames"),
    cloudEnhanced: () => t("library.status.cloudEnhanced"),
    failed: () => t("library.status.failedRetry"),
    waitingSync: () => t("library.status.waitingSync"),
    waitingModels: () => t("download.waiting"),
    recording: () => t("library.status.recording"),
  }[status]();
  const base = cn(
    "inline-flex h-[26px] items-center gap-1.5 rounded-full px-2.5 text-[12px] font-semibold whitespace-nowrap",
    cls,
    className,
  );
  const content = (
    <>
      <Icon name={icon} size={15} className={status === "processing" ? "animate-spin motion-reduce:animate-none" : undefined} />
      {label}
    </>
  );
  if (status === "failed" && onRetry) {
    return (
      <button type="button" onClick={onRetry} data-status={status} className={cn(base, "hover:brightness-95")}>
        {content}
      </button>
    );
  }
  return (
    <span data-status={status} className={base}>
      {content}
    </span>
  );
}
