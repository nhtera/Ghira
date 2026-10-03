// SPDX-License-Identifier: Apache-2.0
// One library row (design D3): time, title with a one-line summary, people,
// source icon + duration and the status pill. The library list adds selection
// and quick actions; search results use the same row with highlighted text.
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { formatTime, type Locale } from "@ghi/i18n";
import { Avatar, Icon, StatusPill, cn, type IconName } from "@ghi/ui";
import type { MeetingRow } from "../../bindings";
import { RowChips } from "../folders/row-chips";
import { durationLabel } from "./duration";
import { inProgress, type RowStatus } from "./meeting-status";

const KIND_ICON: Record<string, IconName> = { call: "videocam", room: "groups", mobile: "mobile", import: "upload_file" };
export const KINDS = ["call", "room", "mobile", "import"] as const;
/** The core says `live`/`file` in `source` and `call`/`room` in `mode`. */
export const kindOf = (row: MeetingRow) => (row.source === "file" || row.source === "import" ? "import" : row.source === "mobile" ? "mobile" : row.mode);

const SHOWN_PEOPLE = 4;

export type MeetingRowViewProps = {
  row: MeetingRow;
  /** `null`: no pill (a row built from a search hit knows nothing of the meeting's state). */
  status: RowStatus | null;
  /** The title, plain or with highlighted matches. */
  title: ReactNode;
  /** The second line: the notes' summary, or the best search snippet. */
  line?: ReactNode;
  locale: Locale;
  folderName?: string;
  onOpen: () => void;
  /** Makes a failed row's pill the Retry button. */
  onRetry?: () => void;
  /** Multi-select: the checkbox takes the time's place on hover or while selecting. */
  selection?: { selected: boolean; selecting: boolean; label: string; onPick: (shift: boolean) => void };
  /** Quick actions shown over the pill on hover or focus. */
  actions?: ReactNode;
  /** Below the row (the delete confirmation). */
  below?: ReactNode;
};

export function MeetingRowView({ row, status, title, line, locale, folderName, onOpen, onRetry, selection, actions, below }: MeetingRowViewProps) {
  const { t } = useTranslation();
  const kind = KINDS.find((k) => k === kindOf(row));
  const selected = selection?.selected ?? false;
  // Not while recording or being processed: the length is still growing or unknown.
  const showDuration = row.durationMs != null && row.durationMs > 0 && row.status !== "recording" && !inProgress(row.status);
  const failed = status?.status === "failed";
  const boxShown = selection?.selecting || selected;
  return (
    <div
      data-status={status?.status}
      data-selected={selected ? "true" : undefined}
      className={cn(
        "group relative grid grid-cols-[minmax(0,1fr)_210px] items-center rounded-row hover:bg-surface2 focus-within:bg-surface2",
        selected && "bg-accent-soft hover:bg-accent-soft",
      )}
    >
      <button
        type="button"
        onClick={onOpen}
        className="grid min-h-[62px] min-w-0 grid-cols-[64px_minmax(0,1fr)_auto_112px] items-center gap-3.5 rounded-row py-2.5 pr-0 pl-3 text-left"
      >
        <span
          className={cn(
            "text-mono text-[12px] whitespace-nowrap text-faint",
            selection && (boxShown ? "invisible" : "group-hover:invisible group-focus-within:invisible"),
          )}
        >
          {row.startedAt != null && formatTime(row.startedAt, locale)}
        </span>
        <span className="grid min-w-0 gap-px">
          <span className="truncate text-[14px] font-semibold text-ink">{title}</span>
          {line != null && line !== "" && <span className="truncate font-serif text-[14.5px] text-muted">{line}</span>}
          <RowChips row={row} folderName={folderName} />
        </span>
        <span className="flex items-center">
          {row.people.slice(0, SHOWN_PEOPLE).map((p) => (
            <Avatar
              key={p.name}
              name={p.name}
              colorSlot={p.colorSlot}
              size="md"
              label={p.name}
              className="-ml-1.5 border-2 border-surface first:ml-0"
            />
          ))}
          {row.people.length > SHOWN_PEOPLE && <Avatar kind="group" count={row.people.length - SHOWN_PEOPLE} size="md" className="-ml-1.5 border-2 border-surface" />}
        </span>
        <span className="flex items-center gap-1.5 text-[12px] text-muted">
          <Icon name={(kind && KIND_ICON[kind]) || "graphic_eq"} size={17} className="flex-none" />
          {kind && <span className="sr-only">{t(`library.sources.${kind}`)}</span>}
          <span className="text-mono whitespace-nowrap">{showDuration && durationLabel(t, row.durationMs!)}</span>
        </span>
      </button>
      {/* Beside the row button, not inside it: the Retry pill is a button of its own. */}
      <span className="flex justify-self-end pr-3">
        {status && <StatusPill status={status.status} percent={status.percent} onRetry={failed ? onRetry : undefined} />}
      </span>
      {selection && (
        <button
          type="button"
          role="checkbox"
          aria-checked={selected}
          aria-label={selection.label}
          onClick={(e) => selection.onPick(e.shiftKey)}
          className={cn(
            "absolute top-1/2 left-3 grid size-6 -translate-y-1/2 place-items-center rounded-seg text-muted",
            !boxShown && "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
          )}
        >
          <Icon name={selected ? "check_box" : "check_box_outline_blank"} size={20} className={selected ? "text-accent" : undefined} />
        </button>
      )}
      {actions && !selection?.selecting && (
        <div
          className={cn(
            "absolute top-1/2 hidden -translate-y-1/2 gap-0.5 rounded-[9px] border border-line2 bg-surface p-[3px] shadow-float group-hover:flex group-focus-within:flex",
            // The pill stays visible and clickable: a failed row's Retry is on it.
            failed ? "right-[226px]" : "right-3",
          )}
        >
          {actions}
        </div>
      )}
      {below && <div className="col-span-2">{below}</div>}
    </div>
  );
}
