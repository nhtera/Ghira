// SPDX-License-Identifier: Apache-2.0
// The star on an item that covers a moment the user marked while recording
// ("You marked this at 12:04"). The words are there for screen readers and as
// the tooltip; the star is the sighted cue.
import { formatClock } from "@ghi/i18n";
import { Icon } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import { useNotesContext } from "./notes-context";
import { marksCovering } from "./notes-model";

export function MarkStar({ gid }: { gid: string }) {
  const { t } = useTranslation();
  const { marks = [] } = useNotesContext();
  const covered = marksCovering(marks, gid);
  if (!covered.length) return null;
  const text = covered.map((m) => t("notes.markedAt", { time: formatClock(m.tMs ?? 0, { pad: true }) })).join(" · ");
  return (
    <span data-testid="mark-star" title={text} className="ml-1.5 inline-flex items-center align-[2px] text-warn">
      <Icon name="star" size={14} />
      <span className="sr-only">{text}</span>
    </span>
  );
}
