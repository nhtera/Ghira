// SPDX-License-Identifier: Apache-2.0
// Topic rail (D6): for long meetings, the topics beside the transcript with
// their times. Click jumps there (the parent scrolls and seeks); the topic
// being discussed is marked while playing.
import { formatClock } from "@ghi/i18n";
import { memo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Icon, cn } from "@ghi/ui";
import type { TopicView } from "../../bindings";
import { usePlayer } from "../../state/player";

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

export const TopicRail = memo(function TopicRail({ topics, onJump }: RailProps) {
  const { t } = useTranslation();
  const now = useNowTopic(topics);
  return (
    <nav aria-label={t("detail.topics.title")} data-testid="topic-rail" className="sticky top-12 flex max-h-[calc(100vh-16rem)] w-56 shrink-0 flex-col gap-1 self-start overflow-y-auto border-l border-line py-3 pr-1 pl-3 max-[1099px]:hidden">
      <h3 className="m-0 text-[13px] font-semibold">{t("detail.topics.title")}</h3>
      <p className="m-0 mb-1 text-[12px] text-muted">{t("detail.topics.subtitle")}</p>
      <ol className="m-0 flex list-none flex-col gap-0.5 p-0">
        {topics.map((topic, i) => (
          <li key={i}>
            <button
              type="button"
              aria-current={i === now ? "true" : undefined}
              onClick={() => topic.tMs != null && onJump(topic.tMs)}
              className={cn("flex w-full flex-col items-start gap-0.5 rounded-ctl border-l-2 px-2 py-1.5 text-left hover:bg-sunk", i === now ? "border-accent bg-accent-soft" : "border-transparent")}
            >
              <span className="text-mono text-[11px] text-muted">{formatClock(topic.tMs ?? 0, { pad: true })}</span>
              <span className="text-[13px] leading-snug">{topic.title}</span>
            </button>
          </li>
        ))}
      </ol>
    </nav>
  );
});
