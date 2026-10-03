// SPDX-License-Identifier: Apache-2.0
// D3 list: meetings grouped by day, one row each with status, people and quick
// actions on hover or focus (design rationale #7), a checkbox column for
// multi-select (shift-click ranges) and a virtualized body so 1,200+ rows scroll
// smoothly. Search results and filters live beside it (search/, filter-bar).
import { observeElementRect, useVirtualizer } from "@tanstack/react-virtual";
import { useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { Locale } from "@ghi/i18n";
import { Icon, InlineConfirm, cn, usePlatform, type IconName } from "@ghi/ui";
import type { MeetingRow } from "../../bindings";
import { groupByDay } from "./group-by-day";
import { useGroupTitle } from "./group-title";
import { inProgress, rowStatus } from "./meeting-status";
import { MeetingRowView } from "./meeting-row";
import { addRange, rangeIds, toggleSelected } from "./selection";

const HEAD_H = 36;
const ROW_H = 62;
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

// A day heading rides on the first row of its day, inside that row's list item (a list may only hold list items).
type Item = { row: MeetingRow; heading?: string };

export function LibraryList({
  rows,
  progress = {},
  needsNames,
  onOpen,
  onDelete,
  onRetry,
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
          const isSelected = selected?.has(row.gid) ?? false;
          const busy = row.status === "recording" || inProgress(row.status);
          return (
            <div key={v.key} role="listitem" data-index={v.index} ref={virt.measureElement} style={pos} className="absolute top-0 left-0 w-full">
              {it.heading && <h2 className="m-0 mt-3 mb-1 ml-3 text-[11.5px] font-semibold tracking-[.06em] text-faint uppercase">{it.heading}</h2>}
              <MeetingRowView
                row={row}
                status={status}
                title={row.title || t("live.titlePlaceholder")}
                line={row.summary}
                locale={locale}
                folderName={row.folder ? folderNames?.[row.folder] : undefined}
                onOpen={() => onOpen(row.gid)}
                onRetry={onRetry ? () => onRetry(row.gid) : undefined}
                selection={
                  onSelectionChange
                    ? { selected: isSelected, selecting, label: t("library.select", { title: row.title || t("live.titlePlaceholder") }), onPick: (shift) => pick(row.gid, shift) }
                    : undefined
                }
                actions={
                  <>
                    <RowAction icon="open_in_new" label={t("common.open")} onClick={() => onOpen(row.gid)} />
                    <RowAction icon="content_copy" label={t("library.quick.copyNotes")} disabled={!onCopyNotes || busy} onClick={() => onCopyNotes?.(row.gid)} />
                    <RowAction
                      icon="delete"
                      label={t("common.delete")}
                      hint={busy ? t("library.deleteBusy") : undefined}
                      disabled={!onDelete || busy}
                      danger
                      onClick={() => setConfirming(row.gid)}
                    />
                  </>
                }
                below={
                  confirming === row.gid && (
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
                  )
                }
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}

function RowAction({ icon, label, hint, onClick, disabled, danger }: { icon: IconName; label: string; hint?: string; onClick?: () => void; disabled?: boolean; danger?: boolean }) {
  return (
    <button
      type="button"
      title={hint}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "flex h-7 items-center gap-1 rounded-seg px-[9px] text-[12px] font-medium whitespace-nowrap hover:bg-surface2 disabled:cursor-not-allowed disabled:opacity-40",
        danger ? "text-rec" : "text-ink",
      )}
    >
      <Icon name={icon} size={16} />
      {label}
    </button>
  );
}
