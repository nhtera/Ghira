// SPDX-License-Identifier: Apache-2.0
// "Your notes": each line you wrote with the app's cited points under it (or
// "not found in the transcript"), then an empty line to keep typing. Enter at
// the end of a line goes to the next; Backspace in an empty one removes it.
import { useTranslation } from "react-i18next";
import { Icon } from "@ghi/ui";
import { useRef } from "react";
import { AutoTextarea } from "./auto-textarea";
import { BlockRow, Provenance } from "./block-row";
import { useNotesContext } from "./notes-context";
import type { UserNote } from "./notes-model";

const TAG_ICON = {
  decision: "flag",
  action: "check_circle",
  question: "help",
} as const;

export function YourNotes({ notes }: { notes: UserNote[] }) {
  const { t } = useTranslation();
  const { edit } = useNotesContext();
  const root = useRef<HTMLDivElement>(null);
  const focusIn = (selector: string) =>
    root.current
      ?.querySelector<HTMLTextAreaElement>(`${selector} textarea`)
      ?.focus();
  const lastGid = notes[notes.length - 1]?.block.gid;

  return (
    <div ref={root} className="flex flex-col gap-4">
      {notes.map(({ block, enhanced }, i) => {
        const tag =
          block.kind in TAG_ICON ? (block.kind as keyof typeof TAG_ICON) : null;
        return (
          <div key={block.gid} className="flex flex-col gap-2">
            {tag && (
              <span className="inline-flex items-center gap-1 self-start rounded-full bg-sunk px-2 py-0.5 text-[11.5px] font-semibold text-muted">
                <Icon name={TAG_ICON[tag]} size={13} />
                {t(`notes.tags.${tag}`)}
              </span>
            )}
            <BlockRow
              block={block}
              label={t("notes.yourNoteLabel")}
              onEnter={() => focusIn("[data-composer]")}
              onBackspaceEmpty={() => {
                const prev = notes[i - 1]?.block.gid;
                void edit.deleteBlock(block.gid);
                focusIn(prev ? `[data-block="${prev}"]` : "[data-composer]");
              }}
            />
            {enhanced && (
              <div className="ml-4 border-l-2 border-line2 pl-3">
                {enhanced.text ? (
                  <BlockRow block={enhanced} label={t("notes.enhancedLabel")} />
                ) : (
                  <Provenance kind="missing" />
                )}
              </div>
            )}
          </div>
        );
      })}
      <div data-composer>
        <AutoTextarea
          value=""
          clearOnEnter
          label={t("live.notepad.label")}
          placeholder={t("live.notepad.placeholder")}
          className="font-serif text-notes text-ink font-bold"
          onCommit={(text) => {
            const v = text.trim();
            if (v) void edit.addBlock(v);
          }}
          onEnter={() => undefined}
          onBackspaceEmpty={() => {
            if (lastGid) focusIn(`[data-block="${lastGid}"]`);
          }}
        />
      </div>
    </div>
  );
}
