// SPDX-License-Identifier: Apache-2.0
// Find in the transcript (⌘F / Ctrl+F inside the tab): accent-insensitive
// ("nhan dien" finds "nhận diện"), next / previous, a count.
import { forwardRef } from "react";
import { useTranslation } from "react-i18next";
import { Icon } from "@ghi/ui";

type Props = {
  query: string;
  onQuery: (q: string) => void;
  /** Total matches and the 0-based current one. */
  count: number;
  current: number;
  onStep: (dir: 1 | -1) => void;
  onEscape: () => void;
};

const stepBtn = "grid size-7 place-items-center rounded-seg text-muted hover:bg-sunk hover:text-ink disabled:opacity-40";

export const FindBar = forwardRef<HTMLInputElement, Props>(function FindBar({ query, onQuery, count, current, onStep, onEscape }, ref) {
  const { t } = useTranslation();
  const has = query.trim().length > 0;
  return (
    <div role="search" className="flex h-9 max-w-[360px] items-center gap-1 rounded-ctl border border-ctl bg-surface px-3 focus-within:border-accent">
      <Icon name="search" size={16} className="shrink-0 text-muted" />
      <input
        ref={ref}
        type="search"
        value={query}
        placeholder={t("detail.findPlaceholder")}
        aria-label={t("detail.findPlaceholder")}
        onChange={(e) => onQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing) return;
          if (e.key === "Enter") {
            e.preventDefault();
            onStep(e.shiftKey ? -1 : 1);
          } else if (e.key === "Escape") {
            e.preventDefault();
            onEscape();
          }
        }}
        className="min-w-0 flex-1 bg-transparent text-[13px] text-ink outline-none placeholder:text-muted"
      />
      {/* The steppers appear with the first search: an empty box is just a box. */}
      {has && (
        <>
          <span role="status" className="shrink-0 text-[12px] text-muted tabular-nums">
            {count ? t("transcript.findCount", { current: current + 1, total: count }) : t("transcript.noMatches")}
          </span>
          <button type="button" disabled={!count} onClick={() => onStep(-1)} aria-label={t("transcript.findPrev")} className={stepBtn}>
            <Icon name="expand_less" size={18} />
          </button>
          <button type="button" disabled={!count} onClick={() => onStep(1)} aria-label={t("transcript.findNext")} className={stepBtn}>
            <Icon name="expand_more" size={18} />
          </button>
        </>
      )}
    </div>
  );
});
