// SPDX-License-Identifier: Apache-2.0
// Always-visible privacy state (brief §7): what is true right now about where
// this meeting's data goes. Icon + text, never color alone.
import { useTranslation } from "react-i18next";
import { Icon, type IconName } from "../../icons/icon";
import { cn } from "../../utils/cn";

export type PrivacyState = "local" | "recording" | "paused" | "cloudMeeting" | "cloudToday";

const LOOK: Record<PrivacyState, { icon: IconName; tone: string; key: "privacy.local" | "privacy.recording" | "privacy.paused" | "privacy.cloudMeeting" | "privacy.cloudToday" }> = {
  local: { icon: "lock", tone: "bg-accent-soft text-accent", key: "privacy.local" },
  recording: { icon: "lock", tone: "bg-rec-soft text-rec-ink", key: "privacy.recording" },
  paused: { icon: "pause", tone: "bg-sunk text-muted", key: "privacy.paused" },
  cloudMeeting: { icon: "cloud", tone: "bg-warn-soft text-warn", key: "privacy.cloudMeeting" },
  cloudToday: { icon: "cloud", tone: "bg-warn-soft text-warn", key: "privacy.cloudToday" },
};

export type PrivacyIndicatorProps = { state: PrivacyState; className?: string };

export function PrivacyIndicator({ state, className }: PrivacyIndicatorProps) {
  const { t } = useTranslation();
  const look = LOOK[state];
  return (
    <span
      role="status"
      data-state={state}
      className={cn("inline-flex h-[26px] items-center gap-1.5 rounded-full px-2.5 text-[12px] font-semibold whitespace-nowrap", look.tone, className)}
    >
      {state === "recording" && <span aria-hidden="true" className="size-[7px] rounded-full bg-rec" />}
      <Icon name={look.icon} size={15} />
      {t(look.key)}
    </span>
  );
}
