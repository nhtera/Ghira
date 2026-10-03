// SPDX-License-Identifier: Apache-2.0
// D3 list: meetings grouped by day, one row each with status, people and quick
// actions on hover or focus (design rationale #7), a checkbox column for
// multi-select (shift-click ranges) and a virtualized body so 1,200+ rows scroll
// smoothly. Search results and filters live beside it (search/, filter-bar).
import { observeElementRect, useVirtualizer } from "@tanstack/react-virtual";
import { useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { formatClock, formatTime, type Locale } from "@ghi/i18n";
import { Avatar, Icon, InlineConfirm, StatusPill, cn, usePlatform, type IconName } from "@ghi/ui";
import type { MeetingRow } from "../../bindings";
import { RowChips } from "../folders/row-chips";
import { groupByDay, type DayGroupKey } from "./group-by-day";
import { rowStatus } from "./meeting-status";
import { addRange, rangeIds, toggleSelected } from "./selection";

const KIND_ICON: Record<string, IconName> = { call: "videocam", room: "groups", mobile: "mobile", import: "upload_file" };
const KINDS = ["call", "room", "mobile", "import"] as const;
/** The core says `live`/`file` in `source` and `call`/`room` in `mode`. */
const kindOf = (row: MeetingRow) => (row.source === "file" || row.source === "import" ? "import" : row.source === "mobile" ? "mobile" : row.mode);
const HEAD_H = 40;
const ROW_H = 56;
// A zero-height viewport (first layout, tests) would render nothing: assume a window's worth.
const FALLBACK_H = 800;

export type LibraryListProps = {
  rows: MeetingRow[];
  /** Live job progress by meeting (0..1), from core events. */
  progress?: Record<string, number | undefined>;
  /** Meetings that finished with voices still unnamed. */
  needsNames?: ReadonlySet<string>;
  onOpen: (meeting: string) => void;
  /** Called after the user confirmed; the list asks first. */
  onDelete?: (meeting: string) => void;
  now?: Date;
  /** Runs a failed meeting's jobs again. */
  onRetry?: (gid: string) => void;
  onExport?: (meeting: string) => void;
  /** Copy the notes as Markdown (the action is disabled without it). */
  onCopyNotes?: (meeting: string) => void;
  /** Folder names by gid, for the folder chip on a row. */
  folderNames?: Readonly<Record<string, string>>;
  /** Controlled multi-select; the checkbox column shows when `onSelectionChange` is set. */
  selected?: ReadonlySet<string>;
  onSelectionChange?: (next: Set<string>) => void;
  /** Scrolls with the list (processing panels, naming cards). */
  header?: ReactNode;
};

function useGroupTitle() {
  const { t, i18n } = useTranslation();
  const tag = i18n.language === "vi" ? "vi-VN" : "en-US";
  return (k: DayGroupKey): string => {
    switch (k.kind) {
      case "today":
        return t("common.today");
      case "yesterday":
        return t("common.yesterday");
      case "lastWeek":
        return t("library.groups.lastWeek");
      case "weekday":
        // 2024-01-07 was a Sunday: map getDay() onto a known date.
        return new Intl.DateTimeFormat(tag, { weekday: "long" }).format(new Date(2024, 0, 7 + k.day));
      case "month":
        return new Intl.DateTimeFormat(tag, {
          month: "long",
          year: "numeric",
        }).format(new Date(k.year, k.month, 1));
      case "unknown":
        return "";
    }
  };
}

// A day heading rides on the first row of its day, inside that row's list item (a list may only hold list items).
type Item = { row: MeetingRow; heading?: string };

export function LibraryList({
  rows,
  progress = {},
  needsNames,
  onOpen,
  onDelete,
  onRetry,
  onExport,
  onCopyNotes,
  selected,
  onSelectionChange,
  folderNames,
  header,
  now,
}: LibraryListProps) {
  const { t, i18n } = useTranslation();
  const locale = (i18n.language === "vi" ? "vi" : "en") as Locale;
  const platform = usePlatform();
  const [confirming, setConfirming] = useState<string | null>(null);
  const title = useGroupTitle();
  const anchor = useRef<string | null>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const [margin, setMargin] = useState(0);
  // The list starts below the header (panels, cards): the virtualizer needs that offset.
  // Runs every render on purpose: the header above can change height at any time.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useLayoutEffect(() => {
    const top = list.current?.offsetTop ?? 0;
    if (top !== margin) setMargin(top);
  });

  const items = useMemo<Item[]>(
    () => groupByDay(rows, now).flatMap((g) => g.rows.map((row, i) => ({ row, heading: i === 0 ? title(g.key) || undefined : undefined }))),
    // `title` is rebuilt every render but only depends on the language.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [rows, now, i18n.language, t],
  );
  const order = useMemo(() => rows.map((r) => r.gid), [rows]);
  const estimateSize = (i: number) => ROW_H + (items[i]?.heading ? HEAD_H : 0);
  const virt = useVirtualizer({
    count: items.length,
    getScrollElement: () => scroller.current,
    estimateSize,
    // A row measured at 0 (not laid out yet, or no layout engine) keeps its estimate, so the window stays a window.
    measureElement: (el, entry) => {
      const h = Math.round(entry?.borderBoxSize?.[0]?.blockSize ?? el.getBoundingClientRect().height);
      return h > 0 ? h : estimateSize(Number(el.getAttribute("data-index")));
    },
    overscan: 10,
    observeElementRect: (inst, cb) => observeElementRect(inst, (r) => cb(r.height > 0 ? r : { ...r, height: FALLBACK_H })),
    scrollMargin: margin,
    getItemKey: (i) => {
      return items[i]!.row.gid;
    },
  });

  const pick = (id: string, shift: boolean) => {
    if (!onSelectionChange) return;
    const cur = selected ?? new Set<string>();
    onSelectionChange(shift ? addRange(cur, rangeIds(order, anchor.current, id)) : toggleSelected(cur, id));
    anchor.current = id;
  };
  const selecting = (selected?.size ?? 0) > 0;

  return (
    <div ref={scroller} className="min-h-0 flex-1 overflow-auto">
      {header}
      <div ref={list} role="list" className="relative" style={{ height: virt.getTotalSize() }}>
        {virt.getVirtualItems().map((v) => {
          const it = items[v.index]!;
          const pos = { transform: `translateY(${v.start - margin}px)` };
          const row = it.row;
          const status = rowStatus(row, progress[row.gid], needsNames?.has(row.gid) ?? false);
          const kind = KINDS.find((k) => k === kindOf(row));
          const isSelected = selected?.has(row.gid) ?? false;
          const busy = row.status === "recording" || row.status === "processing";
          return (
            <div key={v.key} role="listitem" data-index={v.index} ref={virt.measureElement} style={pos} className="absolute top-0 left-0 w-full">
              {it.heading && <h2 className="text-small m-0 mt-3 mb-1 px-2 font-semibold text-muted">{it.heading}</h2>}
              <div
                data-status={status.status}
                data-selected={isSelected ? "true" : undefined}
                className={cn(
                  "group relative flex flex-wrap items-center gap-1 rounded-row hover:bg-surface2 focus-within:bg-surface2",
                  isSelected && "bg-accent-soft hover:bg-accent-soft",
                )}
              >
                {onSelectionChange && (
                  <button
                    type="button"
                    role="checkbox"
                    aria-checked={isSelected}
                    aria-label={t("library.select", {
                      title: row.title || t("live.titlePlaceholder"),
                    })}
                    onClick={(e) => pick(row.gid, e.shiftKey)}
                    className={cn(
                      "ml-1 grid size-7 flex-none place-items-center rounded-seg text-muted",
                      !selecting && !isSelected && "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
                    )}
                  >
                    <Icon name={isSelected ? "check_box" : "check_box_outline_blank"} size={20} className={isSelected ? "text-accent" : undefined} />
                  </button>
                )}
                <button
                  type="button"
                  onClick={() => onOpen(row.gid)}
                  className="flex min-h-12 min-w-0 flex-1 items-center gap-3 rounded-row px-2 py-2 text-left"
                >
                  <Icon name={(kind && KIND_ICON[kind]) || "graphic_eq"} size={20} className="flex-none text-muted" />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[13.5px] font-semibold text-ink">{row.title || t("live.titlePlaceholder")}</span>
                    <span className="text-small block text-muted">
                      {row.startedAt != null && formatTime(row.startedAt, locale)}
                      {kind && `${t("common.metaSep")}${t(`library.sources.${kind}`)}`}
                      {row.durationMs != null && `${t("common.metaSep")}${formatClock(row.durationMs)}`}
                    </span>
                    <RowChips row={row} folderName={row.folder ? folderNames?.[row.folder] : undefined} />
                  </span>
                  {row.people.length > 0 && (
                    <span className="flex flex-none items-center">
                      {row.people.slice(0, 3).map((p) => (
                        <Avatar key={p.name} name={p.name} colorSlot={p.colorSlot} size="md" label={p.name} className="-ml-1.5 border-surface first:ml-0" />
                      ))}
                      {row.people.length > 3 && <Avatar kind="group" count={row.people.length - 3} size="md" className="-ml-1.5" />}
                    </span>
                  )}
                  <StatusPill status={status.status} percent={status.percent} />
                </button>
                <div className={cn("flex flex-none items-center gap-0.5 pr-1.5", "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100")}>
                  {status.status === "failed" && onRetry && <RowAction icon="refresh" label={t("common.tryAgain")} onClick={() => onRetry(row.gid)} />}
                  <RowAction icon="open_in_new" label={t("common.open")} onClick={() => onOpen(row.gid)} />
                  <RowAction icon="ios_share" label={t("common.export")} disabled={!onExport || busy} onClick={() => onExport?.(row.gid)} />
                  <RowAction icon="content_copy" label={t("library.quick.copyNotes")} disabled={!onCopyNotes || busy} onClick={() => onCopyNotes?.(row.gid)} />
                  <RowAction
                    icon="delete"
                    label={t("common.delete")}
                    hint={busy ? t("library.deleteBusy") : undefined}
                    disabled={!onDelete || busy}
                    onClick={() => setConfirming(row.gid)}
                  />
                </div>
                {confirming === row.gid && (
                  <InlineConfirm
                    className="w-full"
                    question={t("library.deleteQuestion", {
                      context: platform,
                      count: 1,
                    })}
                    confirmLabel={t("common.delete")}
                    onCancel={() => setConfirming(null)}
                    onConfirm={() => {
                      setConfirming(null);
                      onDelete?.(row.gid);
                    }}
                  />
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function RowAction({ icon, label, hint, onClick, disabled }: { icon: IconName; label: string; hint?: string; onClick?: () => void; disabled?: boolean }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={hint ?? label}
      disabled={disabled}
      onClick={onClick}
      className="grid size-7 place-items-center rounded-seg text-muted hover:bg-sunk hover:text-ink disabled:cursor-not-allowed disabled:opacity-40"
    >
      <Icon name={icon} size={17} />
    </button>
  );
}
