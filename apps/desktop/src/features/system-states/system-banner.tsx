// SPDX-License-Identifier: Apache-2.0
// One app-level notice: a full-width strip under the title bar with an icon, a
// message, an outlined action and (when it can be dismissed) a close button.
// Never a modal: these show while recording too.
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button, Icon, cn, type IconName } from "@ghi/ui";

export type BannerTone = "warn" | "rec" | "info";
const TONE: Record<BannerTone, string> = { warn: "bg-warn-soft text-warn", rec: "bg-rec-soft text-rec-ink", info: "bg-surface2 text-ink" };

/** The banner's action: outlined in the banner's own color, as in the design. */
export const BANNER_ACTION = "h-8.5! rounded-lg! border! border-current! bg-transparent! px-3! text-[13px]! text-current! hover:bg-transparent! hover:brightness-90";

export function SystemBanner({
  id,
  tone = "warn",
  assertive = false,
  icon,
  title,
  children,
  actions,
  onDismiss,
}: {
  id: string;
  tone?: BannerTone;
  /** Announced at once (role alert) whatever the tone. */
  assertive?: boolean;
  icon: IconName;
  title?: ReactNode;
  children?: ReactNode;
  actions?: ReactNode;
  onDismiss?: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div
      role={assertive || tone === "rec" ? "alert" : "status"}
      data-banner={id}
      className={cn("flex flex-wrap items-center gap-x-2.5 gap-y-2 border-b border-line py-2 pr-3.5 pl-4 text-[13px] font-medium", TONE[tone])}
    >
      <Icon name={icon} size={18} className="flex-none" />
      <div className="min-w-0 flex-1">
        {title && <b className="block font-semibold">{title}</b>}
        {children && <span className="block">{children}</span>}
      </div>
      <div className="flex flex-wrap items-center gap-2">{actions}</div>
      {onDismiss && (
        <Button size="sm" variant="ghost" icon="close" aria-label={t("system.dismiss")} className="size-7 text-current hover:bg-transparent hover:brightness-90" onClick={onDismiss} />
      )}
    </div>
  );
}
