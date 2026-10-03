// SPDX-License-Identifier: Apache-2.0
// Onboarding permissions row for Calendar, optional (phase 14d). macOS only:
// elsewhere a calendar file is imported in Settings → Recording.
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { APP_NAME } from "@ghi/i18n";
import { Button, Icon, useToast, usePlatform } from "@ghi/ui";
import { ipc } from "../../ipc";
import { Notice } from "../onboarding/step-frame";
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
    <li className="grid grid-cols-[28px_minmax(0,1fr)_auto] items-start gap-x-3 gap-y-1 border-b border-line py-3 last:border-b-0">
      <Icon name="event_upcoming" size={22} className="mt-px text-muted" />
      <div className="min-w-0">
        <b className="text-[14px]">
          {t("onboarding.permissions.calendar.title")}
        </b>
        <p className="m-0 text-[13px] leading-normal text-muted">
          {t("onboarding.permissions.calendar.body")}
        </p>
        {state === "denied" && (
          <div className="mt-2 flex flex-col gap-2">
            <Notice tone="warn" icon="warning">
              {t("calendar.denied_mac", { app: APP_NAME })}
            </Notice>
            <Button
              size="sm"
              variant="secondary"
              className="self-start"
              onClick={() => void ipc.commands.openPrivacySettings("calendars")}
            >
              {t("common.openSystemSettings_mac")}
            </Button>
          </div>
        )}
      </div>
      <div className="pt-0.5">
        {state === "authorized" && (
          <span className="flex items-center gap-1.5 text-[13px] font-semibold text-accent">
            <Icon name="check_circle" size={18} />
            {t("onboarding.permissions.granted")}
          </span>
        )}
        {state === "notDetermined" && (
          <Button size="sm" onClick={() => void allow()}>
            {t("onboarding.permissions.allow")}
          </Button>
        )}
        {state === "denied" && (
          <span className="flex items-center gap-1.5 text-[13px] font-semibold text-warn">
            <Icon name="warning" size={18} />
            {t("onboarding.permissions.denied")}
          </span>
        )}
      </div>
    </li>
  );
}
