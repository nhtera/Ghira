// SPDX-License-Identifier: Apache-2.0
// Action item (brief §7): checkbox, text, owner, due, citations. Done, due
// soon and overdue are shown with strike-through / icon + text, not color alone.
import * as Checkbox from "@radix-ui/react-checkbox";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { Icon } from "../../icons/icon";
import { cn } from "../../utils/cn";
import { Avatar } from "../avatar";
import { CitationChip } from "../citation-chip";
import type { NoteCitation } from "../note-block";

export type ActionOwner = { name: string; colorSlot: number; isMe?: boolean };
export type ActionDue = { /** Already formatted ("~2 weeks", "Due tomorrow"). */ text: string; tone?: "normal" | "soon" | "overdue" };

export type ActionItemProps = {
  text: string;
  done?: boolean;
  /** `null`/missing: unassigned. */
  owner?: ActionOwner | null;
  due?: ActionDue;
  citations?: NoteCitation[];
  onToggle?: (done: boolean) => void;
  onCite?: (index: number, citation: NoteCitation) => void;
  className?: string;
};

export function ActionItem({ text, done = false, owner, due, citations = [], onToggle, onCite, className }: ActionItemProps) {
  const { t } = useTranslation();
  const textId = useId();
  const tone = due?.tone ?? "normal";

  return (
    <div data-done={done ? "true" : undefined} className={cn("flex items-start gap-2 py-1", className)}>
      <Checkbox.Root
        checked={done}
        onCheckedChange={(v) => onToggle?.(v === true)}
        aria-labelledby={textId}
        className="group grid size-6 shrink-0 place-items-center rounded-seg"
      >
        <span className="grid size-[18px] place-items-center rounded-[5px] border-2 border-ctl text-on-accent group-data-[state=checked]:border-accent group-data-[state=checked]:bg-accent">
          <Checkbox.Indicator>
            <Icon name="check" size={14} />
          </Checkbox.Indicator>
        </span>
      </Checkbox.Root>
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-3 gap-y-1">
        <span id={textId} className={cn("min-w-0 flex-1 basis-56 text-body", done ? "text-muted line-through" : "text-ink")}>
          {text}
        </span>
        {citations.map((c, i) => (
          <CitationChip key={i} {...c} onClick={onCite ? () => onCite(i, c) : undefined} />
        ))}
        <span className="inline-flex items-center gap-1.5 text-[12.5px] text-muted">
          {owner ? <Avatar kind={owner.isMe ? "me" : "person"} name={owner.name} colorSlot={owner.colorSlot} size="md" /> : <Avatar kind="unknown" size="md" />}
          {owner ? owner.name : t("notes.unassigned")}
        </span>
        {due && due.text && (
          <span data-tone={tone} className={cn("inline-flex items-center gap-1 text-[12.5px]", tone === "soon" && "font-semibold text-warn", tone === "overdue" && "font-semibold text-rec-ink", tone === "normal" && "text-muted")}>
            {tone === "soon" && <Icon name="schedule" size={14} />}
            {tone === "overdue" && <Icon name="error" size={14} />}
            {due.text}
          </span>
        )}
      </div>
    </div>
  );
}
