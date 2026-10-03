// SPDX-License-Identifier: Apache-2.0
// D3 filter chips: People, Source, Template (multi-choice) and Date (one
// preset). A chip shows "Label · n" while it filters, like the design.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Icon, Popover, cn, type IconName } from "@ghi/ui";
import { hasFilters, toggleIn, type DatePreset, type LibraryFilters, type SourceKind } from "./filters";

// [multi off, multi on, single off, single on]
const BOX: IconName[] = ["check_box_outline_blank", "check_box", "radio_button_unchecked", "radio_button_checked"];

type Option = { value: string; label: string };

function Chip({
  label,
  count,
  options,
  selected,
  onToggle,
  single,
}: {
  label: string;
  count: number;
  options: Option[];
  selected: readonly string[];
  onToggle: (v: string) => void;
  single?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const on = count > 0;
  return (
    <Popover
      open={open}
      onOpenChange={setOpen}
      label={label}
      className="flex min-w-48 flex-col gap-0.5 p-1.5"
      trigger={
        <button
          type="button"
          className={cn(
            "inline-flex h-7 items-center gap-0.5 rounded-full border pr-1.5 pl-2.5 text-[12.5px]",
            on ? "border-accent bg-accent-soft font-semibold text-accent" : "border-ctl bg-surface font-medium text-muted",
          )}
        >
          {on ? `${label} · ${count}` : label}
          <Icon name="expand_more" size={16} />
        </button>
      }
    >
      {options.length === 0 && <span className="text-small px-2 py-1.5 text-muted">{"–"}</span>}
      {options.map((o) => {
        const checked = selected.includes(o.value);
        const icon = BOX[(single ? 2 : 0) + (checked ? 1 : 0)]!;
        return (
          <button
            key={o.value}
            type="button"
            role={single ? "menuitemradio" : "menuitemcheckbox"}
            aria-checked={checked}
            onClick={() => {
              onToggle(o.value);
              if (single) setOpen(false);
            }}
            className="flex h-8 items-center gap-2 rounded-seg px-2 text-left text-[13px] whitespace-nowrap hover:bg-surface2"
          >
            <Icon name={icon} size={18} className={checked ? "text-accent" : "text-muted"} />
            {o.label}
          </button>
        );
      })}
    </Popover>
  );
}

export function FilterBar({
  filters,
  onChange,
  people,
  templates,
  folders = [],
  tags = [],
  onClear,
}: {
  filters: LibraryFilters;
  onChange: (f: LibraryFilters) => void;
  people: string[];
  templates: Option[];
  /** Folders and tags that exist (the chips are hidden while there are none). */
  folders?: Option[];
  tags?: Option[];
  onClear: () => void;
}) {
  const { t } = useTranslation();
  const sources: Option[] = [
    { value: "live", label: t("library.sourceLive") },
    { value: "file", label: t("library.sources.import") },
  ];
  const dates: Option[] = [
    { value: "today", label: t("common.today") },
    { value: "week", label: t("library.dates.week") },
    { value: "month", label: t("library.dates.month") },
  ];
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <Chip
        label={t("library.filters.people")}
        count={filters.people.length}
        options={people.map((p) => ({ value: p, label: p }))}
        selected={filters.people}
        onToggle={(v) => onChange({ ...filters, people: toggleIn(filters.people, v) })}
      />
      <Chip
        label={t("library.filters.source")}
        count={filters.source.length}
        options={sources}
        selected={filters.source}
        onToggle={(v) =>
          onChange({
            ...filters,
            source: toggleIn(filters.source, v as SourceKind),
          })
        }
      />
      <Chip
        label={t("library.filters.template")}
        count={filters.template.length}
        options={templates}
        selected={filters.template}
        onToggle={(v) => onChange({ ...filters, template: toggleIn(filters.template, v) })}
      />
      {(folders.length > 0 || filters.folder != null) && (
        <Chip
          single
          label={t("library.filters.folder")}
          count={filters.folder != null ? 1 : 0}
          options={[{ value: "", label: t("organize.noFolder") }, ...folders]}
          selected={filters.folder != null ? [filters.folder] : []}
          onToggle={(v) => onChange({ ...filters, folder: filters.folder === v ? null : v })}
        />
      )}
      {(tags.length > 0 || filters.tags.length > 0) && (
        <Chip
          label={t("library.filters.tags")}
          count={filters.tags.length}
          options={tags}
          selected={filters.tags}
          onToggle={(v) => onChange({ ...filters, tags: toggleIn(filters.tags, v) })}
        />
      )}
      <Chip
        single
        label={t("library.filters.date")}
        count={filters.date ? 1 : 0}
        options={dates}
        selected={filters.date ? [filters.date] : []}
        onToggle={(v) =>
          onChange({
            ...filters,
            date: filters.date === v ? null : (v as DatePreset),
          })
        }
      />
      {hasFilters(filters) && (
        <button type="button" onClick={onClear} className="h-7 rounded-seg px-2.5 text-[12.5px] font-semibold text-accent hover:bg-sunk">
          {t("library.clearFilters")}
        </button>
      )}
    </div>
  );
}
