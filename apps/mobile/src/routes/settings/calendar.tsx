// SPDX-License-Identifier: Apache-2.0
// Settings → Calendar: connect the phone's calendar so a recording started
// during an event is named after it. Off until the user connects; iOS asks
// the first time, a "no" points to Settings. Turning it off stops reading
// events (iOS keeps the permission until the user withdraws it).
import { ListRow, ListSection } from "@ghi/ui";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ipc } from "../../ipc";
import { Btn, ErrorLine } from "../../features/settings/controls";
import { Page } from "../../features/settings/page";
import { useCalendar } from "../../features/calendar";

export function CalendarScreen() {
  const { t } = useTranslation();
  const cal = useCalendar();
  const [turnedOff, setTurnedOff] = useState(false);
  const status = cal.status;
  const connected = status?.connected ?? false;

  return (
    <Page title={t("mobile.settings.calendar.title")} back="settings" error={cal.loadError} onRetry={cal.reload}>
      {status && (
        <>
          <ErrorLine code={cal.error} fallback="mobile.settings.saveFailed" />
          <ListSection header={t("mobile.settings.calendar.header")} footer={t("mobile.settings.calendar.footer")}>
            <ListRow
              title={t("mobile.settings.calendar.title")}
              subtitle={connected ? t("mobile.settings.calendar.statusConnected") : t("mobile.settings.calendar.statusOff")}
              icon="calendar_today"
              value={connected ? t("mobile.settings.calendar.connected") : t("mobile.settings.calendar.off")}
            />
          </ListSection>
          <div className="mx-4 flex flex-col items-start gap-3">
            {status.access === "unavailable" && <p className="text-ios-subhead m-0 text-muted">{t("mobile.settings.calendar.unavailable")}</p>}
            {status.access === "denied" && (
              <>
                <p role="status" className="text-ios-subhead m-0 text-muted">
                  {t("mobile.settings.calendar.denied")}
                </p>
                <Btn tone="primary" onClick={() => void ipc.commands.openAppSettings()}>
                  {t("mobile.settings.calendar.openSettings")}
                </Btn>
              </>
            )}
            {(status.access === "notDetermined" || (status.access === "authorized" && !connected)) && (
              <Btn
                tone="primary"
                disabled={cal.busy}
                onClick={() =>
                  void cal.connect().then(() => {
                    setTurnedOff(false);
                  })
                }
              >
                {t("mobile.settings.calendar.connect")}
              </Btn>
            )}
            {connected && (
              <Btn
                disabled={cal.busy}
                onClick={() =>
                  void cal.disconnect().then((done) => {
                    if (done) setTurnedOff(true);
                  })
                }
              >
                {t("mobile.settings.calendar.disconnect")}
              </Btn>
            )}
            {turnedOff && !connected && (
              <p role="status" className="text-ios-footnote m-0 text-muted">
                {t("mobile.settings.calendar.disconnected")}
              </p>
            )}
          </div>
        </>
      )}
    </Page>
  );
}
