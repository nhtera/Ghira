// SPDX-License-Identifier: Apache-2.0
// One editable block with its provenance (doc 02 §D): what you wrote, what
// the app wrote from the transcript, what you edited (kept on regenerate), or
// a jot with no match in the transcript. Same look as @ghi/ui NoteBlock, with
// a textarea in place of the paragraph and the citations under it.
import { Icon, cn } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import type { NoteBlockView } from "../../bindings";
import { CitationLink } from "../citation/citation-link";
import { AutoTextarea } from "./auto-textarea";
import { useNotesContext } from "./notes-context";

export type BlockRowKind = "user" | "ai" | "edited" | "missing";

export const kindOf = (b: NoteBlockView): BlockRowKind =>
  b.origin === "user" ? "user" : b.origin === "aiEdited" ? "edited" : "ai";

export function Provenance({ kind }: { kind: BlockRowKind }) {
  const { t } = useTranslation();
  const label = {
    user: [t("notes.youWrote"), "person"],
    ai: [t("notes.writtenByApp"), "auto_awesome"],
    edited: [t("notes.editedKept"), "bookmark"],
    missing: [t("notes.notFound"), "search"],
  } as const;
  const [text, icon] = label[kind];
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 text-[11.5px] leading-4",
        kind === "missing" ? "text-[12.5px] text-warn" : "text-muted",
      )}
    >
      <Icon name={icon} size={kind === "missing" ? 15 : 13} />
      {text}
    </span>
  );
}

export type BlockRowProps = {
  block: NoteBlockView;
  /** Overrides the provenance derived from `block.origin` (a jot with no match). */
  kind?: BlockRowKind;
  label: string;
  autoFocus?: boolean;
  onEnter?: () => void;
  onBackspaceEmpty?: () => void;
  className?: string;
};

export function BlockRow({
  block,
  kind = kindOf(block),
  label,
  autoFocus,
  onEnter,
  onBackspaceEmpty,
  className,
}: BlockRowProps) {
  const { speakers, audioAvailable, edit } = useNotesContext();
  const commit = (text: string) => {
    const v = text.trim();
    if (v === block.text) return;
    // Emptying a line you wrote deletes it; an AI block can't be emptied (it keeps its text).
    if (!v)
      return block.origin === "user"
        ? void edit.deleteBlock(block.gid)
        : undefined;
    void edit.editBlock(block.gid, v);
  };
  return (
    <div
      data-kind={kind}
      data-block={block.gid}
      className={cn("flex flex-col gap-1", className)}
    >
      <AutoTextarea
        value={block.text}
        label={label}
        onCommit={commit}
        onEnter={onEnter}
        onBackspaceEmpty={onBackspaceEmpty}
        autoFocus={autoFocus}
        keepOnEmpty={block.origin !== "user"}
        className={cn(
          "font-serif text-notes",
          kind === "ai" ? "text-ai" : "text-ink",
          kind === "user" && "font-bold",
        )}
      />
      {block.citations.length > 0 && (
        <span className="flex flex-wrap gap-1.5">
          {block.citations.map((c, i) => (
            <CitationLink
              key={i}
              citation={c}
              speakers={speakers}
              audioAvailable={audioAvailable}
            />
          ))}
        </span>
      )}
      <Provenance kind={kind} />
    </div>
  );
}
