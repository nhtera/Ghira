// SPDX-License-Identifier: Apache-2.0
// One block of notes (brief §7) with its provenance: what you typed, what the
// app wrote from the transcript, what you edited (pinned), or a jot that has
// no match in the transcript. Text is plain text (RT-6).
import { useTranslation } from "react-i18next";
import { Icon } from "../../icons/icon";
import { cn } from "../../utils/cn";
import { CitationChip, type CitationChipProps } from "../citation-chip";

export type NoteKind = "user" | "ai" | "edited" | "missing";

export type NoteCitation = Pick<CitationChipProps, "timeMs" | "index" | "broken" | "text" | "visited">;

export type NoteBlockProps = {
  kind: NoteKind;
  text: string;
  citations?: NoteCitation[];
  /** A citation chip was activated (index into `citations`). */
  onCite?: (index: number, citation: NoteCitation) => void;
  className?: string;
};

export function NoteBlock({ kind, text, citations = [], onCite, className }: NoteBlockProps) {
  const { t } = useTranslation();
  const missing = kind === "missing";
  const provenance =
    kind === "user" ? (
      <>
        <Icon name="person" size={13} />
        {t("notes.youWrote")}
      </>
    ) : kind === "ai" ? (
      <>
        <Icon name="auto_awesome" size={13} />
        {t("notes.writtenByApp")}
      </>
    ) : kind === "edited" ? (
      <>
        <Icon name="bookmark" size={13} />
        {t("notes.editedKept")}
      </>
    ) : (
      <>
        <Icon name="search" size={15} />
        {t("notes.notFound")}
      </>
    );

  return (
    <div data-kind={kind} className={cn("flex flex-col gap-1", className)}>
      <p className={cn("m-0 font-serif text-notes", kind === "ai" ? "text-ai" : "text-ink", (kind === "user" || missing) && "font-bold")}>
        {text}
        {citations.map((c, i) => (
          <span key={i} className="ml-1.5 inline-block whitespace-nowrap">
            <CitationChip {...c} onClick={onCite ? () => onCite(i, c) : undefined} />
          </span>
        ))}
      </p>
      <span className={cn("inline-flex items-center gap-1 text-[11.5px] leading-4", missing ? "text-[12.5px] text-warn" : "text-muted")}>{provenance}</span>
    </div>
  );
}
