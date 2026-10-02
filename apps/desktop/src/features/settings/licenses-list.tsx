// SPDX-License-Identifier: Apache-2.0
// About → Licenses: a searchable, grouped list; a row opens to its license
// text (a text node in <pre>, RT-6). Above LIST_LIMIT rows the list is
// virtualized so the ~650 entries stay cheap.
import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Icon, cn } from "@ghi/ui";
import { loadLicenses } from "../../generated/licenses";
import { filterLicenses, licenseRows, type LicenseGroup, type LicenseRow } from "./logic";
import { inputCls } from "./parts";

export const LIST_LIMIT = 300;
const GROUPS: LicenseGroup[] = ["models", "assets", "rust", "js"];

type Item = { kind: "head"; group: LicenseGroup; count: number } | { kind: "row"; row: LicenseRow };

export function toItems(rows: LicenseRow[]): Item[] {
  return GROUPS.flatMap((g) => {
    const inGroup = rows.filter((r) => r.group === g);
    return inGroup.length ? [{ kind: "head" as const, group: g, count: inGroup.length }, ...inGroup.map((row) => ({ kind: "row" as const, row }))] : [];
  });
}

export function LicensesList({ loader = loadLicenses }: { loader?: typeof loadLicenses }) {
  const { t } = useTranslation();
  const { data } = useQuery({ queryKey: ["licenses"], queryFn: loader, staleTime: Infinity });
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const rows = useMemo(() => (data ? licenseRows(data) : []), [data]);
  const items = useMemo(() => toItems(filterLicenses(rows, query)), [rows, query]);
  const shown = items.filter((i) => i.kind === "row").length;
  const toggle = (id: string) => setOpen((o) => (o === id ? null : id));

  return (
    <div className="flex flex-col gap-2.5">
      <div className="relative max-w-sm">
        <Icon name="search" size={16} className="pointer-events-none absolute top-2 left-2.5 text-muted" />
        <input type="search" className={cn(inputCls, "w-full pl-8")} value={query} placeholder={t("settings.about.search")} aria-label={t("settings.about.search")} onChange={(e) => setQuery(e.target.value)} />
      </div>
      <span role="status" className="text-small text-muted" data-testid="license-count">
        {data ? t("settings.about.count", { count: shown }) : t("settings.about.loading")}
      </span>
      {items.length > LIST_LIMIT ? <Virtual items={items} open={open} onToggle={toggle} /> : <ul className="m-0 flex list-none flex-col p-0">{items.map((it) => <Entry key={it.kind === "head" ? it.group : it.row.id} item={it} open={open} onToggle={toggle} />)}</ul>}
    </div>
  );
}

function Virtual({ items, open, onToggle }: { items: Item[]; open: string | null; onToggle: (id: string) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  // eslint-disable-next-line react-hooks/incompatible-library -- the virtual list is the only consumer of v
  const v = useVirtualizer({ count: items.length, getScrollElement: () => ref.current, estimateSize: () => 36, overscan: 12 });
  return (
    <div ref={ref} className="h-[420px] overflow-auto rounded-xl border border-line" data-testid="license-virtual">
      <ul className="relative m-0 list-none p-0" style={{ height: v.getTotalSize() }}>
        {v.getVirtualItems().map((vi) => {
          const it = items[vi.index]!;
          return (
            <li key={vi.key} data-index={vi.index} ref={v.measureElement} className="absolute top-0 left-0 w-full" style={{ transform: `translateY(${vi.start}px)` }}>
              <Entry item={it} open={open} onToggle={onToggle} bare />
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function Entry({ item, open, onToggle, bare }: { item: Item; open: string | null; onToggle: (id: string) => void; bare?: boolean }) {
  const { t } = useTranslation();
  if (item.kind === "head") {
    const h = <h3 className="text-small m-0 bg-surface2 px-3 py-1.5 font-semibold text-muted">{t(`settings.about.groups.${item.group}`, { count: item.count })}</h3>;
    return bare ? h : <li>{h}</li>;
  }
  const { row } = item;
  const expanded = open === row.id;
  const body = (
    <div className="border-b border-line">
      <button type="button" aria-expanded={expanded} onClick={() => onToggle(row.id)} className="flex min-h-9 w-full items-center gap-2 px-3 text-left text-[13px] hover:bg-sunk">
        <Icon name={expanded ? "expand_more" : "chevron_right"} size={16} className="shrink-0 text-muted" />
        <span className="min-w-0 flex-1 truncate font-medium">{row.name}</span>
        {row.version && <span className="text-small shrink-0 text-muted">{row.version}</span>}
        <span className="text-small shrink-0 text-muted">{row.license}</span>
      </button>
      {expanded && (
        <div className="flex flex-col gap-2 px-4 pb-3">
          {row.url && <p className="text-small m-0 break-all text-muted">{row.url}</p>}
          {row.copyright.length > 0 && <pre className="text-small m-0 font-sans break-words whitespace-pre-wrap text-muted">{row.copyright.join("\n")}</pre>}
          {row.text ? <pre className="m-0 max-h-72 overflow-auto rounded-lg bg-sunk p-3 font-mono text-[11.5px] leading-snug whitespace-pre-wrap">{row.text}</pre> : <p className="text-small m-0 text-muted">{t("settings.about.noText", { license: row.license })}</p>}
        </div>
      )}
    </div>
  );
  return bare ? body : <li>{body}</li>;
}
