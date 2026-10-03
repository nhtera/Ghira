// SPDX-License-Identifier: Apache-2.0
// Settings → About → Open-source software: every model, bundled asset, crate and
// npm package the app ships with its license; a row opens to the license text
// (a text node in <pre>).
import { Icon, ListRow, ListSection } from "@ghi/ui";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Page } from "../../features/settings/page";
import { filterLicenses, GROUPS, useDebounced, useLicenseRows, type LicenseRow } from "../../features/settings/licenses";

export function LicensesScreen() {
  const { t } = useTranslation();
  const all = useLicenseRows();
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const rows = useMemo(() => filterLicenses(all ?? [], query), [all, query]);
  // Announce the count once typing pauses, not on every keystroke.
  const settled = useDebounced(query);
  const announced = useMemo(() => filterLicenses(all ?? [], settled).length, [all, settled]);
  return (
    <Page title={t("mobile.settings.about.licencesTitle")} back="about">
      <div className="px-4 pb-1">
        <div className="flex min-h-ios-target items-center gap-2 rounded-(--ios-radius-group) bg-sunk px-3">
          <Icon name="search" size={20} className="size-5 shrink-0 text-muted" />
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("mobile.settings.about.licencesSearch")}
            aria-label={t("mobile.settings.about.licencesSearch")}
            autoCapitalize="none"
            autoCorrect="off"
            className="text-ios-body min-h-ios-target min-w-0 flex-1 appearance-none bg-transparent text-ink outline-none select-text placeholder:text-muted [&::-webkit-search-cancel-button]:hidden"
          />
        </div>
      </div>
      <p role="status" aria-live="polite" className="text-ios-footnote m-0 px-8 pt-1 text-muted">
        {all ? t("mobile.settings.about.licencesCount", { count: announced }) : t("mobile.settings.about.licencesLoading")}
      </p>
      {GROUPS.map((g) => {
        const inGroup = rows.filter((r) => r.group === g);
        if (!inGroup.length) return null;
        return (
          <ListSection key={g} header={t(`mobile.settings.about.group.${g}`, { count: inGroup.length })}>
            {inGroup.map((r) => (
              <Entry key={r.id} row={r} open={open === r.id} onToggle={() => setOpen((o) => (o === r.id ? null : r.id))} />
            ))}
          </ListSection>
        );
      })}
    </Page>
  );
}

function Entry({ row, open, onToggle }: { row: LicenseRow; open: boolean; onToggle: () => void }) {
  const detailId = `license-${row.id.replace(/[^a-z0-9]/gi, "-")}`;
  return (
    <>
      <ListRow title={row.version ? `${row.name} ${row.version}` : row.name} value={row.license} onPress={onToggle} expanded={open} controls={detailId} />
      {open && (
        <li id={detailId} className="list-none px-4 pb-3">
          {row.copyright.map((c) => (
            <p key={c} className="text-ios-footnote m-0 pb-1 break-words text-muted select-text">
              {c}
            </p>
          ))}
          {row.text && (
            <pre tabIndex={0} className="text-ios-caption2 m-0 max-h-72 overflow-auto font-mono break-words whitespace-pre-wrap text-ink-2 select-text">
              {row.text}
            </pre>
          )}
        </li>
      )}
    </>
  );
}
