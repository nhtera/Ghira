// SPDX-License-Identifier: Apache-2.0
// M4 Actions tab: action items with a done toggle, owner and citations.
import { ActionItem } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import type { Citation, MeetingNotes, MeetingSpeaker } from "../../bindings";
import { speakerOf, toNoteCitation } from "./notes-model";

export type ActionsPanelProps = {
  notes: MeetingNotes;
  speakers: MeetingSpeaker[];
  visited: ReadonlySet<string>;
  onToggle: (item: string, done: boolean) => void;
  onCite: (citation: Citation, key: string) => void;
};

export function ActionsPanel({
  notes,
  speakers,
  visited,
  onToggle,
  onCite,
}: ActionsPanelProps) {
  const { t } = useTranslation();
  if (notes.actionItems.length === 0) {
    return (
      <p className="text-ios-subhead m-0 px-6 py-10 text-center text-muted">
        {t("mobile.detail.actionsEmpty")}
      </p>
    );
  }
  return (
    <ul className="m-0 flex list-none flex-col gap-1 px-4 py-3">
      {notes.actionItems.map((a) => {
        const owner = speakerOf(speakers, a.ownerSpeakerGid);
        return (
          <li key={a.gid} className="min-h-ios-target">
            <ActionItem
              text={a.text}
              done={a.done}
              owner={
                owner
                  ? {
                      name:
                        owner.name ??
                        t("speakers.numbered", { number: owner.number }),
                      colorSlot: owner.colorSlot,
                      isMe: owner.isMe,
                    }
                  : null
              }
              due={a.dueText ? { text: a.dueText } : undefined}
              citations={a.citations.map((c, i) =>
                toNoteCitation(c, i, visited, `${a.gid}:${i}`),
              )}
              onToggle={(done) => onToggle(a.gid, done)}
              onCite={(i) => onCite(a.citations[i], `${a.gid}:${i}`)}
            />
          </li>
        );
      })}
    </ul>
  );
}
