// SPDX-License-Identifier: Apache-2.0
// Citation chip: the link from a note sentence to the transcript moment it
// came from (brief §7). Shows a time (mono) or the source index; ≥24×24.
import { formatClock } from "@ghi/i18n";
import { useTranslation } from "react-i18next";
import { Icon } from "../../icons/icon";
import { cn } from "../../utils/cn";

export type CitationChipProps = {
  /** Moment in the meeting; shown as a clock time. */
  timeMs?: number;
  /** Source number, when there is no time to show. */
  index?: number;
  /** The audio was deleted: dashed, the transcript line still exists. */
  broken?: boolean;
  /** Visible text instead of the time (a date for a broken source). */
  text?: string;
  /** The reader already followed it (app state). */
  visited?: boolean;
  onClick?: () => void;
  className?: string;
};

export function CitationChip({ timeMs, index, broken, text: shownText, visited, onClick, className }: CitationChipProps) {
  const { t } = useTranslation();
  const time = timeMs != null ? formatClock(timeMs, { pad: true }) : undefined;
  const text = shownText ?? time ?? String(index ?? "");
  const name =
    time != null
      ? t(broken ? "citation.showAtBroken" : "citation.showAt", { time })
      : t(broken ? "citation.showIndexBroken" : "citation.showIndex", { index: index ?? 0 });
  return (
    <button
      type="button"
      aria-label={name}
      data-state={broken ? "broken" : visited ? "visited" : "default"}
      onClick={onClick}
      className={cn(
        "inline-flex h-6 min-w-6 shrink-0 items-center justify-center gap-0.5 rounded-seg border bg-surface2 pr-[7px] pl-1 align-middle text-mono text-[12px]",
        "transition-colors duration-(--motion-fast) ease-out hover:border-accent hover:text-accent",
        broken ? "border-dashed border-line2 bg-transparent text-muted" : visited ? "border-line2 text-accent" : "border-line2 text-muted",
        className,
      )}
    >
      {broken ? <Icon name="link_off" size={14} /> : visited ? <Icon name="check" size={14} /> : <Icon name="play_arrow" size={14} />}
      {text}
    </button>
  );
}
