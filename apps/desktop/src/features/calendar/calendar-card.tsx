// SPDX-License-Identifier: Apache-2.0
// Settings → Recording card: connect Calendar (macOS) or an .ics file, and the global ask switch (phase 14d).
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { APP_NAME } from "@ghi/i18n";
import { Button, Icon, usePlatform, useToast } from "@ghi/ui";
import { ipc } from "../../ipc";
import { Card, Note, Row, SwitchRow } from "../settings/parts";
import {
  calendarError,
  calendarStatusKey,
  useCalendarStatus,
} from "./use-calendar";

const ICS_ERRORS = new Set(["icsInvalid", "icsTooLarge"]);

export function CalendarCard() {
  const { t } = useTranslation();
  const context = usePlatform();
  const { show } = useToast();
  const client = useQueryClient();
  const { data: status } = useCalendarStatus();
  const [icsError, setIcsError] = useState(false);
  if (!status) return null;

  const refresh = () => client.invalidateQueries({ queryKey: ["calendar"] });
  const fail = (code: string) =>
    show({ tone: "warning", title: calendarError(t, code) });
  const setAskOnStart = async (askOnStart: boolean) => {
    const r = await ipc.commands.setCalendar({ askOnStart });
    if (r.status === "error") fail(r.error);
    void refresh();
  };
  const connect = async () => {
    const r = await ipc.commands.requestCalendarAccess();
    if (r.status === "error") fail(r.error);
    else client.setQueryData(calendarStatusKey, r.data);
    void refresh();
  };
  const importIcs = async () => {
    setIcsError(false);
    const r = await ipc.commands.pickIcsFile();
    if (r.status === "error") {
      if (ICS_ERRORS.has(r.error)) setIcsError(true);
      else fail(r.error);
    }
    void refresh();
  };
  const removeIcs = async () => {
    setIcsError(false);
    const r = await ipc.commands.removeIcsFile();
    if (r.status === "error") fail(r.error);
    void refresh();
  };

  const eventkit = status.eventkit;
  const connected = eventkit === "authorized" || status.ics != null;
  return (
    <Card title={t("calendar.title")} hint={t(`calendar.body_${context}`)}>
      {eventkit !== "unavailable" && (
        <Row
          label={t("calendar.title")}
          hint={
            eventkit === "denied"
              ? t("calendar.denied_mac", { app: APP_NAME })
              : undefined
          }
        >
          {eventkit === "authorized" && (
            <span className="text-small flex items-center gap-1.5 font-semibold text-accent">
              <Icon name="check_circle" size={18} />
              {t("calendar.connected")}
            </span>
          )}
          {eventkit === "notDetermined" && (
            <Button onClick={() => void connect()}>
              {t("calendar.connect")}
            </Button>
          )}
          {eventkit === "denied" && (
            <Button
              onClick={() => void ipc.commands.openPrivacySettings("calendars")}
            >
              {t(`common.openSystemSettings_${context}`)}
            </Button>
          )}
        </Row>
      )}
      <Row
        label={
          status.ics
            ? t("calendar.icsFrom", { name: status.ics.name })
            : t("calendar.importIcs")
        }
        hint={status.ics ? t("calendar.icsHint", { app: APP_NAME }) : undefined}
      >
        {status.ics ? (
          <Button onClick={() => void removeIcs()}>
            {t("calendar.icsRemove")}
          </Button>
        ) : (
          <Button onClick={() => void importIcs()}>
            {t("calendar.importIcs")}
          </Button>
        )}
      </Row>
      {icsError && (
        <Note icon="warning" tone="warn">
          {t("calendar.icsInvalid", { app: APP_NAME })}
        </Note>
      )}
      {connected && (
        <SwitchRow
          label={t("calendar.askOnStart")}
          checked={status.askOnStart}
          onChange={(v) => void setAskOnStart(v)}
        />
      )}
    </Card>
  );
}
