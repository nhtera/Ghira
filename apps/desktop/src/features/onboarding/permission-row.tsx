// SPDX-License-Identifier: Apache-2.0
// One row of the onboarding permissions step, as in the design: icon, name and
// reason, and on the right either the "Allow…" button or the row's state in its
// place. A "Open System Settings" button sits under the state only when the
// system has been told no (or can't say).
import { createContext, useContext, useEffect, useRef, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button, Icon, cn, usePlatform, type IconName } from "@ghi/ui";

/** The row's name, so its buttons can say what they are for ("Allow Microphone"). */
const RowName = createContext("");

/** The outlined accent "Allow…" button. */
export function AllowButton({ onClick, disabled, busy }: { onClick: () => void; disabled?: boolean; busy?: boolean }) {
  const { t } = useTranslation();
  const name = useContext(RowName);
  return (
    <Button variant="secondary" aria-label={t("onboarding.permissions.allowFor", { name })} className="h-8! rounded-lg! border-accent! px-3.5! text-accent!" disabled={disabled || busy} onClick={onClick}>
      {t("onboarding.permissions.allow")}
    </Button>
  );
}

export function PermissionStatus({ tone, icon, children }: { tone: "ok" | "warn" | "muted"; icon: IconName; children: ReactNode }) {
  return (
    <span className={cn("flex items-center gap-1.5 text-[13px] font-semibold", tone === "ok" ? "text-accent" : tone === "warn" ? "text-warn" : "text-muted")}>
      <Icon name={icon} size={18} />
      {children}
    </span>
  );
}

/** "Open System Settings", the same label and style in every row. */
export function OpenSettingsButton({ onClick }: { onClick: () => void }) {
  const { t } = useTranslation();
  const context = usePlatform();
  const name = useContext(RowName);
  return (
    <Button size="sm" variant="secondary" aria-label={t(`onboarding.permissions.openSettingsFor_${context}`, { name })} onClick={onClick}>
      {t(`common.openSystemSettings_${context}`)}
    </Button>
  );
}

export function PermissionRow({ icon, title, why, status, action }: { icon: IconName; title: string; why: string; status: ReactNode; action?: ReactNode }) {
  const acted = useRef(false);
  const statusRef = useRef<HTMLDivElement>(null);
  // "Allow…" is replaced by the state: keep the keyboard where it was and say what happened.
  const asking = isAskingNode(status);
  const wasAsking = useRef(asking);
  useEffect(() => {
    if (wasAsking.current && !asking && acted.current) statusRef.current?.focus();
    wasAsking.current = asking;
  }, [asking]);
  return (
    <RowName.Provider value={title}>
      <li onClickCapture={() => (acted.current = true)} className="grid grid-cols-[32px_minmax(0,1fr)_auto] items-start gap-x-3 gap-y-1 border-b border-line py-3.5">
        <Icon name={icon} size={22} className="mt-px text-muted" />
        <div className="min-w-0">
          <b className="text-[14px]">{title}</b>
          <p className="m-0 text-[13px] leading-normal text-muted">{why}</p>
        </div>
        <div className="flex flex-col items-end gap-2 pt-0.5">
          <div ref={statusRef} role="status" tabIndex={-1} className="rounded outline-none focus-visible:ring-2 focus-visible:ring-accent">
            {status}
          </div>
          {action}
        </div>
      </li>
    </RowName.Provider>
  );
}

/** True while the status slot still holds the Allow button. */
function isAskingNode(status: ReactNode): boolean {
  return typeof status === "object" && status !== null && "type" in status && status.type === AllowButton;
}
