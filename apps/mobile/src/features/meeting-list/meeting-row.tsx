// SPDX-License-Identifier: Apache-2.0
// One meeting in the list: title, when, a one-line summary, people (color +
// initial) and the sync chip. Swipe left (or focus the Delete button) to delete,
// with an inline confirm.
import { SensitiveChip } from "../sensitive";
import { Avatar, InlineConfirm, SyncChip, type SyncChipKind } from "@ghi/ui";
import { useRef, useState, type TouchEvent } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingRow } from "../../bindings";
import { LOCKED_EVENT } from "../app-lock/events";
import { useWindowEvent } from "../meeting-view/use-window-event";
import { useMeetingWhen } from "./format";

/** Width (px) of the revealed Delete button. */
const REVEAL = 96;
const MAX_PEOPLE = 3;

export type MeetingRowViewProps = {
  row: MeetingRow;
  chip: SyncChipKind | undefined;
  /** The paired computer's name, for "Final pass on <device>". */
  device?: string;
  onOpen: () => void;
  onRetry: () => void;
  onDelete: () => void;
};

export function MeetingRowView({
  row,
  chip,
  device,
  onOpen,
  onRetry,
  onDelete,
}: MeetingRowViewProps) {
  const { t } = useTranslation();
  const when = useMeetingWhen();
  const title = row.title || t("mobile.meetings.untitled");
  const [open, setOpen] = useState(false);
  const [drag, setDrag] = useState<number | null>(null);
  const [asking, setAsking] = useState(false);
  // Locking hides the list: an open confirm or a swiped row closes.
  useWindowEvent(LOCKED_EVENT, () => {
    setAsking(false);
    setOpen(false);
  });
  const from = useRef<{ x: number; y: number; base: number } | null>(null);
  // WebKit blurs a button on press: that must not slide the row back over it.
  const pressing = useRef(false);

  const start = (e: TouchEvent) => {
    const touch = e.touches[0];
    from.current = {
      x: touch.clientX,
      y: touch.clientY,
      base: open ? -REVEAL : 0,
    };
  };
  const move = (e: TouchEvent) => {
    const f = from.current;
    if (!f) return;
    const dx = e.touches[0].clientX - f.x;
    // A mostly vertical drag is a scroll.
    if (Math.abs(e.touches[0].clientY - f.y) > Math.abs(dx)) {
      from.current = null;
      setDrag(null);
      return;
    }
    setDrag(Math.min(0, Math.max(-REVEAL, f.base + dx)));
  };
  const end = () => {
    if (from.current && drag !== null) setOpen(drag < -REVEAL / 2);
    from.current = null;
    setDrag(null);
  };

  const offset = drag ?? (open ? -REVEAL : 0);
  if (asking) {
    return (
      <div className="px-4 py-2">
        <InlineConfirm
          question={t("mobile.meetings.delete.question", { title })}
          confirmLabel={t("mobile.meetings.delete.confirm")}
          onConfirm={() => {
            setAsking(false);
            onDelete();
          }}
          onCancel={() => {
            setAsking(false);
            setOpen(false);
          }}
        />
      </div>
    );
  }

  const shown = row.people.slice(0, MAX_PEOPLE);
  const more = row.people.length - shown.length;
  return (
    <div
      data-meeting={row.gid}
      className="relative overflow-hidden border-b border-line"
    >
      {/* Invisible (not hidden: it stays focusable) until the row slides off it. */}
      <div
        className="absolute inset-y-0 end-0 flex"
        style={{ width: REVEAL, opacity: offset < 0 ? 1 : 0 }}
      >
        <button
          type="button"
          onClick={() => setAsking(true)}
          onFocus={() => setOpen(true)}
          onPointerDown={() => (pressing.current = true)}
          onBlur={() => {
            if (!pressing.current) setOpen(false);
            pressing.current = false;
          }}
          className="text-ios-body min-h-ios-target w-full bg-rec font-semibold text-on-accent"
        >
          {t("mobile.meetings.delete.action")}
          <span className="sr-only">{`: ${title}`}</span>
        </button>
      </div>
      <div
        onTouchStart={start}
        onTouchMove={move}
        onTouchEnd={end}
        onTouchCancel={end}
        style={{ transform: `translateX(${offset}px)` }}
        className={`relative bg-surface ${drag === null ? "transition-transform duration-(--motion-base) motion-reduce:transition-none" : ""}`}
      >
        <button
          type="button"
          onClick={onOpen}
          className="block min-h-ios-target w-full px-4 pt-3 text-start"
        >
          <span className="text-ios-body block font-bold break-words">
            {title}
          </span>
          <span className="text-ios-footnote mt-0.5 block text-muted">
            {when(row.startedAt, row.durationMs)}
          </span>
          {row.summary && (
            <span className="text-ios-subhead mt-0.5 block text-muted [display:-webkit-box] overflow-hidden [-webkit-box-orient:vertical] [-webkit-line-clamp:2]">
              {row.summary}
            </span>
          )}
        </button>
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 px-4 pt-1.5 pb-3">
          {chip && (
            <SyncChip
              chip={chip}
              device={device}
              onRetry={chip.kind === "failed" ? onRetry : undefined}
            />
          )}
          {row.sensitive && <SensitiveChip />}
          {shown.length > 0 && (
            <span
              className="ms-auto flex items-center gap-1"
              role="group"
              aria-label={shown.map((p) => p.name).join(", ")}
            >
              {shown.map((p, i) => (
                <Avatar
                  key={`${i}:${p.name}`}
                  name={p.name}
                  colorSlot={p.colorSlot}
                  size="md"
                />
              ))}
              {more > 0 && <Avatar kind="group" count={more} size="md" />}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}
