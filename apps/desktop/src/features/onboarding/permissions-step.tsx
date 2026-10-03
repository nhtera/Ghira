// SPDX-License-Identifier: Apache-2.0
// D1 step 4: each permission is explained before its OS prompt (priming): the
// "Allow…" button is the only thing that triggers the system dialog.
import { CalendarPermissionRow } from "../calendar/calendar-permission-row";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button, Icon, cn, useToast, usePlatform, type IconName } from "@ghi/ui";
import type { Permission } from "../../bindings";
import { ipc } from "../../ipc";
import { Notice, StepActions, StepFrame, type StepNav } from "./step-frame";

/** The mic permission; re-read when the window regains focus (the user may have flipped it in System Settings). */
export function useMicPermission() {
  const { t } = useTranslation();
  const { show } = useToast();
  const [state, setState] = useState<Permission | null>(null);
  useEffect(() => {
    let alive = true;
    const read = () => void ipc.commands.micPermission().then((p) => alive && setState(p));
    read();
    window.addEventListener("focus", read);
    return () => {
      alive = false;
      window.removeEventListener("focus", read);
    };
  }, []);
  const request = useCallback(async () => {
    const r = await ipc.commands.requestMicPermission();
    if (r.status === "ok") setState(r.data);
    else show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
  }, [show, t]);
  return { state, request };
}

function Row({ icon, title, why, status, children }: { icon: IconName; title: string; why: string; status: ReactNode; children?: ReactNode }) {
  return (
    <li className="grid grid-cols-[28px_minmax(0,1fr)_auto] items-start gap-x-3 gap-y-1 border-b border-line py-3 last:border-b-0">
      <Icon name={icon} size={22} className="mt-px text-muted" />
      <div className="min-w-0">
        <b className="text-[14px]">{title}</b>
        <p className="m-0 text-[13px] leading-normal text-muted">{why}</p>
        {children && <div className="mt-2 flex flex-col gap-2">{children}</div>}
      </div>
      <div className="pt-0.5">{status}</div>
    </li>
  );
}

function Status({ tone, icon, children }: { tone: "ok" | "warn" | "muted"; icon: IconName; children: ReactNode }) {
  return (
    <span className={cn("flex items-center gap-1.5 text-[13px] font-semibold", tone === "ok" ? "text-accent" : tone === "warn" ? "text-warn" : "text-muted")}>
      <Icon name={icon} size={18} />
      {children}
    </span>
  );
}

export function PermissionsStep({ nav }: { nav: StepNav }) {
  const { t } = useTranslation();
  const { show } = useToast();
  const openPane = async (pane: "microphone" | "systemAudio") => {
    const r = await ipc.commands.openPrivacySettings(pane);
    if (r.status === "error") show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
  };
  const context = usePlatform();
  const mic = useMicPermission();
  const micBlocked = mic.state === "denied" || mic.state === "restricted";

  return (
    <StepFrame title={t("onboarding.permissions.title")} body={t("onboarding.permissions.body", { context })}>
      <ul className="m-0 flex list-none flex-col p-0">
        <Row
          icon="mic"
          title={t("onboarding.permissions.microphone.title")}
          why={t("onboarding.permissions.microphone.body")}
          status={
            mic.state === "granted" ? (
              <Status tone="ok" icon="check_circle">
                {t("onboarding.permissions.granted")}
              </Status>
            ) : micBlocked ? (
              <Status tone="warn" icon="block">
                {t("onboarding.permissions.denied")}
              </Status>
            ) : (
              <Button variant="secondary" className="border-accent text-accent" onClick={() => void mic.request()} disabled={mic.state === null}>
                {t("onboarding.permissions.allow")}
              </Button>
            )
          }
        >
          {micBlocked && (
            <Notice tone="warn" icon="warning">
              <span className="flex flex-col items-start gap-2">
                {t(`onboarding.permissions.micDenied_${context}`)}
                <Button size="sm" variant="secondary" onClick={() => void openPane("microphone")}>
                  {t(`common.openSystemSettings_${context}`)}
                </Button>
              </span>
            </Notice>
          )}
        </Row>

        <Row
          icon="speaker"
          title={t(`onboarding.permissions.systemAudio.title_${context}`)}
          why={t("onboarding.permissions.systemAudio.body")}
          status={
            context === "mac" ? (
              <Status tone="muted" icon="schedule">
                {t("onboarding.permissions.checkedInTest")}
              </Status>
            ) : (
              <Status tone="ok" icon="check_circle">
                {t("onboarding.permissions.notNeeded")}
              </Status>
            )
          }
        >
          {context === "mac" && (
            <>
              {/* macOS can't be asked whether this is on: the test recording shows it. */}
              <Notice icon="info">{t("onboarding.permissions.systemAudioNote")}</Notice>
              <Button size="sm" className="self-start" onClick={() => void openPane("systemAudio")}>
                {t("common.openSystemSettings_mac")}
              </Button>
            </>
          )}
          <Notice icon="mic">{t("onboarding.permissions.roomOnly")}</Notice>
        </Row>

        <Row
          icon="notifications"
          title={t("onboarding.permissions.notifications.title")}
          why={t("onboarding.permissions.notifications.body")}
          status={
            <Status tone="muted" icon="schedule">
              {t("onboarding.permissions.askedLater")}
            </Status>
          }
        />

        <CalendarPermissionRow />
      </ul>
      <StepActions nav={nav} />
    </StepFrame>
  );
}
