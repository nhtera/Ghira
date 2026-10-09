// SPDX-License-Identifier: Apache-2.0
// One editable block with its provenance (doc 02 §D): what you wrote, what
// the app wrote from the transcript, what you edited (kept on regenerate), or
// a jot with no match in the transcript. Same look as @ghi/ui NoteBlock, with
// a textarea in place of the paragraph and the citations under it.
import { Icon, Menu, cn } from "@ghi/ui";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import type { NoteBlockView } from "../../bindings";
import { CitationGroup } from "../citation/citation-link";
import { MarkStar } from "./mark-star";
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
    edited: [t("notes.editedKept"), "push_pin"],
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
  const { t } = useTranslation();
  const { meeting, speakers, audioAvailable, edit } = useNotesContext();
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
  const proposed = block.kind === "proposal";
  // After a move the menu is not where it was: put focus back on this block's menu button.
  const menuButton = useRef<HTMLButtonElement>(null);
  const refocus = useRef(false);
  useEffect(() => {
    if (refocus.current) {
      refocus.current = false;
      menuButton.current?.focus();
    }
  }, [block.kind]);
  const move = (toProposed: boolean) => {
    refocus.current = true;
    void edit.setDecisionStatus(block.gid, toProposed);
  };
  // The app's own decisions can move between Decided and Proposed.
  const movable = block.origin !== "user" && (block.kind === "decision" || proposed);
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
        trailing={
          (block.citations.length > 0 || kind === "edited") && (
            <>
              {block.pinned && (
                <span title={t("notes.pinned")} className="ml-1.5 inline-flex items-center align-[2px] text-muted">
                  <Icon name="push_pin" size={13} />
                  <span className="sr-only">{t("notes.pinned")}</span>
                </span>
              )}
              <MarkStar gid={block.gid} />
              {block.citations.length > 0 && (
                <span className="ml-1.5 align-[2px]">
                  <CitationGroup citations={block.citations} speakers={speakers} audioAvailable={audioAvailable} meeting={meeting} />
                </span>
              )}
              {/* Edited blocks say so right after the text, like the design. */}
              {kind === "edited" && (
                <span className="ml-2 align-[2px] font-sans">
                  <Provenance kind="edited" />
                </span>
              )}
            </>
          )
        }
        className={cn(
          "font-serif text-notes",
          kind === "ai" ? "text-ai" : "text-ink",
          kind === "user" && "font-bold",
        )}
      />
      {/* The legend above the notes says what the two colors mean; edited and unmatched blocks still say it themselves. */}
      <div className="flex items-center gap-2">
        {proposed && (
          <span data-testid="proposed-chip" className="inline-flex h-5 items-center rounded-seg border border-dashed border-line2 px-1.5 font-sans text-[11.5px] font-semibold text-muted">
            {t("notes.proposed")}
          </span>
        )}
        {kind !== "edited" && (
          <span className={cn((kind === "ai" || kind === "user") && "sr-only")}>
            <Provenance kind={kind} />
          </span>
        )}
        {movable && (
          <Menu
            label={t("notes.decisionMenu")}
            trigger={
              <button ref={menuButton} type="button" aria-label={t("notes.decisionMenu")} className="grid size-6 place-items-center rounded-seg text-muted hover:bg-sunk hover:text-ink">
                <Icon name="more_horiz" size={16} />
              </button>
            }
            items={[
              proposed
                ? { label: t("notes.markDecided"), icon: "check", onSelect: () => move(false) }
                : { label: t("notes.markProposed"), icon: "help", onSelect: () => move(true) },
            ]}
          />
        )}
      </div>
    </div>
  );
}
