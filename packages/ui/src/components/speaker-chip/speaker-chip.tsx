// SPDX-License-Identifier: Apache-2.0
// Speaker chip (brief §7): who is talking, in the order people arrive.
// Color + initial + name, never color alone. A button (aria-pressed for `selected`) when clickable, else plain.
import type { CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { Icon } from "../../icons/icon";
import { cn } from "../../utils/cn";
import { Avatar, initialOf } from "../avatar";

export type SpeakerChipState = "identifying" | "numbered" | "suggested" | "named" | "auto" | "merged";

export type SpeakerChipProps = {
  state: SpeakerChipState;
  /** "Speaker 2", "Sarah"; for `merged` the name merged into. Not needed while identifying. */
  name?: string;
  /** Color slot 1..8 (0 = Others). */
  colorSlot?: number;
  isMe?: boolean;
  /** `suggested`: the name the voice sounds like ("Linh"). */
  suggestion?: string;
  /** `merged`: the label that was merged away ("Speaker 5"). */
  mergedFrom?: string;
  /** This speaker is selected (e.g. its lines are highlighted). */
  selected?: boolean;
  onClick?: () => void;
  onAcceptSuggestion?: () => void;
  className?: string;
};

export function SpeakerChip({ state, name = "", colorSlot = 1, isMe, suggestion, mergedFrom, selected, onClick, onAcceptSuggestion, className }: SpeakerChipProps) {
  const { t } = useTranslation();
  const identifying = state === "identifying";
  const initial = state === "numbered" || state === "suggested" ? (name.match(/\d+\s*$/)?.[0].trim() ?? initialOf(name)) : initialOf(name);
  const shown = identifying ? t("speakers.identifying") : state === "merged" && mergedFrom ? `${mergedFrom} → ${name}` : name;

  const main = "inline-flex h-full items-center gap-1.5 rounded-full text-left whitespace-nowrap";
  const body = (
    <>
      {identifying ? <Avatar kind="unknown" size="md" /> : <Avatar kind={isMe ? "me" : "person"} name={name} initial={initial} colorSlot={colorSlot} size="md" />}
      <span className={cn(identifying ? "text-muted italic" : state === "merged" ? "text-muted" : "text-ink")}>{shown}</span>
      {state === "merged" && <Icon name="call_merge" size={15} className="text-muted" />}
      {state === "auto" && <Icon name="verified" size={15} label={t("speakers.voiceMatch")} className="text-accent" />}
    </>
  );

  return (
    <span
      data-state={state}
      data-selected={selected ? "true" : undefined}
      className={cn(
        "inline-flex h-8 items-center rounded-full border-[1.5px] bg-surface pr-2 pl-[3px] text-[13px] font-medium",
        selected ? "border-(--s-ring) shadow-[0_0_0_3px_var(--warn-soft)]" : "border-line",
        className,
      )}
      style={selected && colorSlot > 0 ? ({ "--s-ring": `var(--s${colorSlot})` } as CSSProperties) : ({ "--s-ring": "var(--accent)" } as CSSProperties)}
    >
      {onClick ? (
        <button type="button" aria-pressed={selected ?? false} onClick={onClick} className={main}>
          {body}
        </button>
      ) : (
        <span className={main}>{body}</span>
      )}
      {state === "suggested" && suggestion && (
        <button
          type="button"
          onClick={onAcceptSuggestion}
          aria-label={t("speakers.acceptSuggestion", { name: suggestion })}
          className="-mr-1 ml-1 inline-flex h-6 items-center gap-0.5 rounded-full bg-warn-soft px-2 text-[12px] font-semibold text-warn"
        >
          {`${suggestion}?`}
          <Icon name="check" size={15} />
        </button>
      )}
    </span>
  );
}
