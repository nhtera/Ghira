// SPDX-License-Identifier: Apache-2.0
// Topic rail (D6): for long meetings, the topics beside the transcript with
// their times. Click jumps there (the parent scrolls and seeks); the topic
// being discussed is marked while playing. The rail sits on the right edge;
// its left edge drags to resize it and a button folds it to a slim strip
// (both per-device prefs).
import { formatClock } from "@ghi/i18n";
import { memo, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { Button, Icon, cn } from "@ghi/ui";
import type { TopicView } from "../../bindings";
import { usePlayer } from "../../state/player";
import { TOPIC_RAIL_WIDTH, clampRailWidth, usePrefs } from "../../state/prefs";

/** A long meeting (an hour or more), one with many topics, or a 20-minute one with at least two gets the rail. */
export const RAIL_MIN_MS = 60 * 60 * 1000;
export const RAIL_MIN_TOPICS = 4;
export const OUTLINE_MIN_MS = 20 * 60 * 1000;
export const OUTLINE_MIN_TOPICS = 2;

export const showTopicRail = (durationMs: number | null | undefined, topics: readonly TopicView[]) =>
  topics.length > 0 &&
  ((durationMs ?? 0) >= RAIL_MIN_MS ||
    topics.length >= RAIL_MIN_TOPICS ||
    (topics.length >= OUTLINE_MIN_TOPICS && (durationMs ?? 0) >= OUTLINE_MIN_MS));

/** Index of the topic being discussed at `ms` (the last that started), or -1. */
export function currentTopic(topics: readonly Pick<TopicView, "tMs">[], ms: number): number {
  let at = -1;
  topics.forEach((t, i) => {
    if (t.tMs != null && t.tMs <= ms) at = i;
  });
  return at;
}

type RailProps = { topics: readonly TopicView[]; onJump: (tMs: number) => void };

const useNowTopic = (topics: readonly TopicView[]) => usePlayer((s) => (s.src ? currentTopic(topics, s.currentMs) : -1));

/** Below 1100 px the rail has no room: the same topics as a collapsible row above the transcript. */
export const TopicOutline = memo(function TopicOutline({ topics, onJump }: RailProps) {
  const { t } = useTranslation();
  const now = useNowTopic(topics);
  const [open, setOpen] = useState(false);
  return (
    <div data-testid="topic-outline" className="border-b border-line min-[1100px]:hidden">
      <button
        type="button"
        aria-expanded={open}
        aria-controls="topic-outline-list"
        onClick={() => setOpen((o) => !o)}
        className="flex h-8 w-full items-center gap-1.5 rounded-seg px-2 text-left text-[13px] font-semibold hover:bg-sunk"
      >
        <Icon name={open ? "expand_more" : "chevron_right"} size={16} />
        {t("detail.topics.outline", { count: topics.length })}
      </button>
      {open && (
        <ol id="topic-outline-list" className="m-0 flex list-none flex-col gap-0.5 p-0 pb-2">
          {topics.map((topic, i) => (
            <li key={i}>
              <button
                type="button"
                aria-current={i === now ? "true" : undefined}
                onClick={() => topic.tMs != null && onJump(topic.tMs)}
                className={cn("flex w-full items-baseline gap-2 rounded-ctl border-l-2 px-2 py-1 text-left hover:bg-sunk", i === now ? "border-accent bg-accent-soft" : "border-transparent")}
              >
                <span className="text-mono text-[11px] text-muted">{formatClock(topic.tMs ?? 0, { pad: true })}</span>
                <span className="text-[13px] leading-snug">{topic.title}</span>
              </button>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
});

/** Arrow keys move the rail's edge by this much; Shift moves it four times as far. */
const KEY_STEP = 16;

export const TopicRail = memo(function TopicRail({ topics, onJump }: RailProps) {
  const { t } = useTranslation();
  const now = useNowTopic(topics);
  const saved = usePrefs((s) => s.topicRailWidth);
  const save = usePrefs((s) => s.setTopicRailWidth);
  const collapsed = usePrefs((s) => s.topicRailCollapsed);
  const setCollapsed = usePrefs((s) => s.setTopicRailCollapsed);
  // While dragging, the width lives here; it is saved once on release.
  const [drag, setDrag] = useState<{ x: number; w: number; now: number } | null>(null);
  const width = drag?.now ?? saved;
  const toggle = t(collapsed ? "detail.topics.show" : "detail.topics.hide");

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? KEY_STEP * 4 : KEY_STEP;
    const to =
      e.key === "ArrowLeft" ? width + step : e.key === "ArrowRight" ? width - step : e.key === "Home" ? TOPIC_RAIL_WIDTH.max : e.key === "End" ? TOPIC_RAIL_WIDTH.min : null;
    if (to == null) return;
    e.preventDefault();
    save(to);
  };

  return (
    <nav
      aria-label={t("detail.topics.title")}
      data-testid="topic-rail"
      data-collapsed={collapsed || undefined}
      style={collapsed ? undefined : { width }}
      className="sticky top-12 ml-auto flex max-h-[calc(100vh-16rem)] shrink-0 self-start max-[1099px]:hidden"
    >
      {/* The rail's left edge: drag (or arrow keys) to resize, double-click for the default width. */}
      {!collapsed && (
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label={t("detail.topics.resize")}
          aria-valuemin={TOPIC_RAIL_WIDTH.min}
          aria-valuemax={TOPIC_RAIL_WIDTH.max}
          aria-valuenow={width}
          tabIndex={0}
          data-testid="topic-rail-handle"
          onKeyDown={onKeyDown}
          onDoubleClick={() => save(TOPIC_RAIL_WIDTH.default)}
          onPointerDown={(e) => {
            if (e.button !== 0) return;
            e.preventDefault();
            e.currentTarget.setPointerCapture?.(e.pointerId);
            setDrag({ x: e.clientX, w: width, now: width });
          }}
          onPointerMove={(e) => drag && setDrag({ ...drag, now: clampRailWidth(drag.w + drag.x - e.clientX) })}
          onPointerUp={() => {
            if (drag) save(drag.now);
            setDrag(null);
          }}
          onPointerCancel={() => setDrag(null)}
          className="group relative z-10 -mr-1 flex w-2 shrink-0 cursor-col-resize touch-none justify-center outline-none select-none"
        >
          <span
            aria-hidden="true"
            className={cn(
              "h-full transition-colors",
              drag ? "w-0.5 bg-accent" : "w-px bg-line group-hover:w-0.5 group-hover:bg-accent group-focus-visible:w-0.5 group-focus-visible:bg-accent",
            )}
          />
        </div>
      )}
      {/* The toggle keeps its place in both states, so it keeps focus when clicked. */}
      <div className={cn("flex min-w-0 flex-1 flex-col overflow-y-auto", collapsed ? "border-l border-line pl-1" : "py-1 pl-2")}>
        <div className="flex items-center gap-1">
          {!collapsed && (
            <h3 title={t("detail.topics.subtitle")} className="m-0 flex-1 truncate px-2 text-[11px] font-semibold tracking-wide text-muted uppercase">
              {t("detail.topics.title")}
            </h3>
          )}
          <Button
            variant="ghost"
            size="sm"
            icon={collapsed ? "right_panel_open" : "right_panel_close"}
            aria-label={toggle}
            title={toggle}
            aria-expanded={!collapsed}
            aria-controls={collapsed ? undefined : "topic-rail-list"}
            onClick={() => setCollapsed(!collapsed)}
            className="shrink-0 text-muted"
          />
        </div>
        {!collapsed && (
          <ol id="topic-rail-list" className="m-0 flex list-none flex-col p-0">
            {topics.map((topic, i) => (
              <li key={i}>
                <button
                  type="button"
                  aria-current={i === now ? "true" : undefined}
                  onClick={() => topic.tMs != null && onJump(topic.tMs)}
                  className={cn("flex w-full items-baseline gap-2 rounded-ctl px-2 py-1 text-left hover:bg-sunk", i === now ? "text-accent" : "text-ink")}
                >
                  <span className={cn("shrink-0 text-mono text-[11px] tabular-nums", i === now ? "text-accent" : "text-muted")}>{formatClock(topic.tMs ?? 0, { pad: true })}</span>
                  <span className="line-clamp-2 min-w-0 text-[13px] leading-snug">{topic.title}</span>
                </button>
              </li>
            ))}
          </ol>
        )}
      </div>
    </nav>
  );
});
