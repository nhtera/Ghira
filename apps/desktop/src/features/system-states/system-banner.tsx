// SPDX-License-Identifier: Apache-2.0
// One app-level notice: an icon, a message, actions, and (when it can be
// dismissed) a close button. Never a modal: these show while recording too.
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button, Icon, cn, type IconName } from "@ghi/ui";

export type BannerTone = "warn" | "rec" | "info";
const TONE: Record<BannerTone, string> = { warn: "bg-warn-soft text-warn", rec: "bg-rec-soft text-rec-ink", info: "bg-surface2 text-ink" };

export function SystemBanner({
  id,
  tone = "warn",
  icon,
  title,
  children,
  actions,
  onDismiss,
}: {
  id: string;
  tone?: BannerTone;
  icon: IconName;
  title?: ReactNode;
  children?: ReactNode;
  actions?: ReactNode;
  onDismiss?: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div
      role={tone === "rec" ? "alert" : "status"}
      data-banner={id}
      className={cn("text-body flex flex-wrap items-start gap-x-2.5 gap-y-2 rounded-row px-3.5 py-2.5", TONE[tone])}
    >
      <Icon name={icon} size={18} className="mt-0.5 flex-none" />
      <div className="min-w-0 flex-1">
        {title && <b className="block font-semibold">{title}</b>}
        {children && <span className="block">{children}</span>}
      </div>
      <div className="flex flex-wrap items-center gap-2">{actions}</div>
      {onDismiss && (
        <Button size="sm" variant="ghost" icon="close" aria-label={t("system.dismiss")} className="-my-0.5 text-current" onClick={onDismiss} />
      )}
    </div>
  );
}
