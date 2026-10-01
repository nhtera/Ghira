// SPDX-License-Identifier: Apache-2.0
// D3 basics: meetings grouped by day, one row each with status and quick
// actions on hover or focus (design rationale #7). Search, filters and
// multi-select are phase 11.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { formatClock, formatTime, type Locale } from "@ghi/i18n";
import { Icon, InlineConfirm, StatusPill, cn, usePlatform, type IconName } from "@ghi/ui";
import type { MeetingRow } from "../../bindings";
import { groupByDay, type DayGroupKey } from "./group-by-day";
import { rowStatus } from "./meeting-status";

const SOURCE_ICON: Record<string, IconName> = { call: "videocam", room: "groups", mobile: "mobile", import: "upload_file" };
const SOURCES = ["call", "room", "mobile", "import"] as const;

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
        return new Intl.DateTimeFormat(tag, { month: "long", year: "numeric" }).format(new Date(k.year, k.month, 1));
      case "unknown":
        return "";
    }
  };
}

export function LibraryList({ rows, progress = {}, needsNames, onOpen, onDelete, onRetry, now }: LibraryListProps) {
  const { t, i18n } = useTranslation();
  const locale = (i18n.language === "vi" ? "vi" : "en") as Locale;
  const platform = usePlatform();
  const [confirming, setConfirming] = useState<string | null>(null);
  const title = useGroupTitle();
  const groups = groupByDay(rows, now);
  return (
    <div className="flex flex-col gap-5">
      {groups.map((g) => {
        const heading = title(g.key);
        return (
          <section key={g.id} aria-label={heading || undefined}>
            {heading && <h2 className="text-small m-0 mb-1 px-2 font-semibold text-muted">{heading}</h2>}
            <ul className="m-0 flex list-none flex-col p-0">
              {g.rows.map((row) => {
                const status = rowStatus(row, progress[row.gid], needsNames?.has(row.gid) ?? false);
                const source = SOURCES.find((s) => s === row.source);
                return (
                  <li key={row.gid} data-status={status.status} className="group relative flex flex-wrap items-center gap-1 rounded-row hover:bg-surface2 focus-within:bg-surface2">
                    <button
                      type="button"
                      onClick={() => onOpen(row.gid)}
                      className="flex min-h-12 min-w-0 flex-1 items-center gap-3 rounded-row px-2 py-2 text-left"
                    >
                      <Icon name={SOURCE_ICON[row.source] ?? "graphic_eq"} size={20} className="flex-none text-muted" />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[13.5px] font-semibold text-ink">{row.title || t("live.titlePlaceholder")}</span>
                        <span className="text-small block text-muted">
                          {row.startedAt != null && formatTime(row.startedAt, locale)}
                          {source && ` · ${t(`library.sources.${source}`)}`}
                          {row.durationMs != null && ` · ${formatClock(row.durationMs)}`}
                        </span>
                      </span>
                      <StatusPill status={status.status} percent={status.percent} />
                    </button>
                    <div className={cn("flex flex-none items-center gap-0.5 pr-1.5", "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100")}>
                      {status.status === "failed" && onRetry && <RowAction icon="refresh" label={t("common.tryAgain")} onClick={() => onRetry(row.gid)} />}
                      <RowAction icon="open_in_new" label={t("common.open")} onClick={() => onOpen(row.gid)} />
                      {/* No notes-to-Markdown command yet (PENDING-ui-library.md). */}
                      <RowAction icon="content_copy" label={t("library.quick.copyNotes")} disabled />
                      <RowAction
                        icon="delete"
                        label={t("common.delete")}
                        hint={row.status === "recording" || row.status === "processing" ? t("library.deleteBusy") : undefined}
                        disabled={!onDelete || row.status === "recording" || row.status === "processing"}
                        onClick={() => setConfirming(row.gid)}
                      />
                    </div>
                    {confirming === row.gid && (
                      <InlineConfirm
                        className="w-full"
                        question={t("library.deleteQuestion", { context: platform, count: 1 })}
                        confirmLabel={t("common.delete")}
                        onCancel={() => setConfirming(null)}
                        onConfirm={() => {
                          setConfirming(null);
                          onDelete?.(row.gid);
                        }}
                      />
                    )}
                  </li>
                );
              })}
            </ul>
          </section>
        );
      })}
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
