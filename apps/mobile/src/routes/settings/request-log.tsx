// SPDX-License-Identifier: Apache-2.0
// Settings → Privacy → Cloud request log: every time notes were sent to a cloud
// provider, newest first. The core keeps no content, so this shows only the
// meeting, provider, model, size and time. The log can't tell a deleted meeting
// from one with no title (both come back empty), so both read "Untitled meeting".
import { ListRow, ListSection } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import { ipc } from "../../ipc";
import { unwrap, useResource } from "../../features/settings/api";
import { Page } from "../../features/settings/page";
import { providerName } from "../../features/settings/providers";

const LIMIT = 200;
const load = async () => unwrap(await ipc.commands.cloudRequestLog(LIMIT));

export function RequestLogScreen() {
  const { t, i18n } = useTranslation();
  const log = useResource(load);
  const when = new Intl.DateTimeFormat(i18n.language === "vi" ? "vi-VN" : "en-US", { dateStyle: "medium", timeStyle: "short" });
  const size = new Intl.NumberFormat(i18n.language);

  return (
    <Page title={t("mobile.privacy.log.title")} back="privacy" error={log.error} onRetry={log.reload}>
      {log.data &&
        (log.data.length === 0 ? (
          <p className="text-ios-footnote mx-4 my-3 text-muted">{t("mobile.privacy.log.empty")}</p>
        ) : (
          <ListSection footer={t("mobile.privacy.log.footer")}>
            {log.data.map((r, i) => (
              <ListRow
                key={`${r.at}-${i}`}
                title={r.meetingTitle || t("mobile.privacy.log.untitled")}
                subtitle={
                  <>
                    <span className="block">
                      {providerName(r.provider)} · {r.model}
                    </span>
                    <span className="block">
                      {t("mobile.privacy.log.tokens", { in: size.format(r.tokensIn ?? 0), out: size.format(r.tokensOut ?? 0) })}
                    </span>
                    <span className="block">{when.format(r.at ?? 0)}</span>
                  </>
                }
              />
            ))}
          </ListSection>
        ))}
    </Page>
  );
}
