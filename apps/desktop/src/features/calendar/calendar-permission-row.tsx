// SPDX-License-Identifier: Apache-2.0
// Onboarding permissions row for Calendar, optional (phase 14d). macOS only:
// elsewhere a calendar file is imported in Settings → Recording.
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { useToast, usePlatform } from "@ghi/ui";
import { ipc } from "../../ipc";
import { AllowButton, OpenSettingsButton, PermissionRow, PermissionStatus } from "../onboarding/permission-row";
import {
  calendarError,
  calendarStatusKey,
  useCalendarStatus,
} from "./use-calendar";

export function CalendarPermissionRow() {
  const { t } = useTranslation();
  const context = usePlatform();
  const { show } = useToast();
  const client = useQueryClient();
  const { data: status } = useCalendarStatus();
  if (context !== "mac" || !status || status.eventkit === "unavailable")
    return null;
  const state = status.eventkit;

  const allow = async () => {
    const r = await ipc.commands.requestCalendarAccess();
    if (r.status === "ok") client.setQueryData(calendarStatusKey, r.data);
    else show({ tone: "warning", title: calendarError(t, r.error) });
    // With access, the events can be read now.
    void client.invalidateQueries({ queryKey: ["calendar", "upcoming"] });
  };
  return (
    <PermissionRow
      icon="event_upcoming"
      title={t("onboarding.permissions.calendar.title")}
      why={t("onboarding.permissions.calendar.body")}
      status={
        state === "authorized" ? (
          <PermissionStatus tone="ok" icon="check_circle">
            {t("onboarding.permissions.granted")}
          </PermissionStatus>
        ) : state === "denied" ? (
          <PermissionStatus tone="warn" icon="block">
            {t("onboarding.permissions.denied")}
          </PermissionStatus>
        ) : (
          <AllowButton onClick={() => void allow()} />
        )
      }
      action={state === "denied" && <OpenSettingsButton onClick={() => void ipc.commands.openPrivacySettings("calendars")} />}
    />
  );
}
