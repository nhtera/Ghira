// SPDX-License-Identifier: Apache-2.0
// Chips under a library row's title: its folder, up to three tags, and the app
// a file came from ("Plaud import"). Plain spans (the row itself is the button).
import { useTranslation } from "react-i18next";
import { Icon } from "@ghi/ui";
import type { MeetingRow } from "../../bindings";
import { sourceAppName } from "./source-app";

const chip = "inline-flex h-5 max-w-40 items-center gap-0.5 rounded-full border border-line2 px-1.5 text-[11.5px] text-muted";
const SHOWN = 3;

export function RowChips({ row, folderName }: { row: Pick<MeetingRow, "folder" | "tags" | "sourceApp">; folderName?: string }) {
  const { t } = useTranslation();
  const app = sourceAppName(row.sourceApp);
  if (!folderName && row.tags.length === 0 && !app) return null;
  return (
    <span className="mt-1 flex flex-wrap items-center gap-1">
      {folderName && (
        <span className={chip}>
          <Icon name="folder" size={12} />
          <span className="truncate">{folderName}</span>
        </span>
      )}
      {row.tags.slice(0, SHOWN).map((x) => (
        <span key={x.gid} className={chip}>
          <Icon name="label" size={12} />
          <span className="truncate">{x.name}</span>
        </span>
      ))}
      {row.tags.length > SHOWN && <span className="text-small text-muted">{t("common.plusCount", { count: row.tags.length - SHOWN })}</span>}
      {app && (
        <span className={chip}>
          <Icon name="upload_file" size={12} />
          {t("library.sourceImport", { source: app })}
        </span>
      )}
    </span>
  );
}
