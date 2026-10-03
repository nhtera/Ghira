// SPDX-License-Identifier: Apache-2.0
// D1 step 4: each permission is explained before its OS prompt (priming): the
// "Allow…" button is the only thing that triggers the system dialog.
import { CalendarPermissionRow } from "../calendar/calendar-permission-row";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Icon, useToast, usePlatform } from "@ghi/ui";
import type { Permission } from "../../bindings";
import { ipc } from "../../ipc";
import { AllowButton, OpenSettingsButton, PermissionRow, PermissionStatus } from "./permission-row";
import { StepActions, StepFrame, type StepNav } from "./step-frame";

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

/** What asking for notifications or probing system audio came back with; null until asked. */
type Asked = "granted" | "denied" | "unknown" | "heard" | "silent" | null;

export function PermissionsStep({ nav }: { nav: StepNav }) {
  const { t } = useTranslation();
  const { show } = useToast();
  const context = usePlatform();
  const mic = useMicPermission();
  const [notifications, setNotifications] = useState<Asked>(null);
  const [systemAudio, setSystemAudio] = useState<Asked>(null);
  const [probing, setProbing] = useState(false);
  const micBlocked = mic.state === "denied" || mic.state === "restricted";

  const failed = (message: string) => show({ tone: "warning", title: t("system.commandFailed", { message }) });
  const openPane = async (pane: "microphone" | "systemAudio" | "notifications") => {
    const r = await ipc.commands.openPrivacySettings(pane);
    if (r.status === "error") failed(r.error);
  };
  const askNotifications = async () => {
    const r = await ipc.commands.requestNotifications();
    if (r.status === "ok") setNotifications(r.data);
    else failed(r.error);
  };
  // A short silent capture of the system audio: that is what makes macOS ask.
  const probeSystemAudio = async () => {
    setProbing(true);
    const r = await ipc.commands.probeSystemAudio();
    setProbing(false);
    if (r.status === "ok") setSystemAudio(r.data);
    else failed(r.error);
  };

  return (
    <StepFrame title={t("onboarding.permissions.title")} body={t("onboarding.permissions.body", { context })}>
      <ul className="m-0 flex list-none flex-col p-0">
        <PermissionRow
          icon="mic"
          title={t("onboarding.permissions.microphone.title")}
          why={t("onboarding.permissions.microphone.body")}
          status={
            mic.state === "granted" ? (
              <PermissionStatus tone="ok" icon="check_circle">
                {t("onboarding.permissions.granted")}
              </PermissionStatus>
            ) : micBlocked ? (
              <PermissionStatus tone="warn" icon="block">
                {t("onboarding.permissions.denied")}
              </PermissionStatus>
            ) : (
              <AllowButton onClick={() => void mic.request()} disabled={mic.state === null} />
            )
          }
          action={micBlocked && <OpenSettingsButton onClick={() => void openPane("microphone")} />}
        />

        <PermissionRow
          icon="speaker"
          title={t(`onboarding.permissions.systemAudio.title_${context}`)}
          why={t("onboarding.permissions.systemAudio.body")}
          status={
            context !== "mac" ? (
              <PermissionStatus tone="ok" icon="check_circle">
                {t("onboarding.permissions.notNeeded")}
              </PermissionStatus>
            ) : systemAudio === "heard" ? (
              <PermissionStatus tone="ok" icon="check_circle">
                {t("onboarding.permissions.granted")}
              </PermissionStatus>
            ) : systemAudio === "denied" ? (
              <PermissionStatus tone="warn" icon="block">
                {t("onboarding.permissions.denied")}
              </PermissionStatus>
            ) : systemAudio ? (
              // macOS can't say whether this is on; the test recording shows it.
              <PermissionStatus tone="muted" icon="schedule">
                {t("onboarding.permissions.confirmedInTest")}
              </PermissionStatus>
            ) : (
              <AllowButton onClick={() => void probeSystemAudio()} busy={probing} />
            )
          }
          action={context === "mac" && (systemAudio === "denied" || systemAudio === "silent" || systemAudio === "unknown") && <OpenSettingsButton onClick={() => void openPane("systemAudio")} />}
        />

        <PermissionRow
          icon="notifications"
          title={t("onboarding.permissions.notifications.title")}
          why={t("onboarding.permissions.notifications.body")}
          status={
            notifications === "granted" ? (
              <PermissionStatus tone="ok" icon="check_circle">
                {t("onboarding.permissions.granted")}
              </PermissionStatus>
            ) : notifications === "denied" ? (
              <PermissionStatus tone="warn" icon="block">
                {t("onboarding.permissions.denied")}
              </PermissionStatus>
            ) : notifications === "unknown" ? (
              <PermissionStatus tone="muted" icon="schedule">
                {t("onboarding.permissions.askedLater")}
              </PermissionStatus>
            ) : (
              <AllowButton onClick={() => void askNotifications()} />
            )
          }
          action={(notifications === "denied" || notifications === "unknown") && <OpenSettingsButton onClick={() => void openPane("notifications")} />}
        />

        <CalendarPermissionRow />
      </ul>
      <p className="m-0 flex items-start gap-2 text-[12.5px] leading-snug text-muted">
        <Icon name="info" size={16} className="mt-px flex-none" />
        {t("onboarding.permissions.roomOnly")}
      </p>
      <StepActions nav={nav} />
    </StepFrame>
  );
}
