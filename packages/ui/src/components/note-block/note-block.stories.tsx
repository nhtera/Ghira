// SPDX-License-Identifier: Apache-2.0
import { useTranslation } from "react-i18next";
import type { ReactNode } from "react";
import { citedMs, pick, sampleNotes } from "../avatar/sample-data";
import type { Story, StoryMeta } from "../../story";
import { NoteBlock } from "./note-block";

export default { title: "Note block", width: 520 } satisfies StoryMeta;

function Sample({ children }: { children: (lang: string) => ReactNode }) {
  const { i18n } = useTranslation();
  return <>{children(i18n.language)}</>;
}

const jot = sampleNotes.yourNotes;

export const User: Story = {
  render: () => (
    <Sample>{(lang) => <NoteBlock kind="user" text={lang === "vi" ? "đổi tên người nói trực tiếp?" : jot[1]!.me!} />}</Sample>
  ),
};
export const AI: Story = {
  render: () => (
    <Sample>
      {(lang) => <NoteBlock kind="ai" text={pick(sampleNotes.summary[1]! as { en: string; vi: string }, lang)} citations={[{ timeMs: citedMs(1) }]} />}
    </Sample>
  ),
};
export const AIEditedByUser: Story = {
  render: () => (
    <Sample>
      {(lang) => {
        const d = sampleNotes.decisions[1]! as { en: string; vi: string };
        return <NoteBlock kind="edited" text={pick(d, lang)} citations={[{ timeMs: citedMs(6) }]} />;
      }}
    </Sample>
  ),
  note: "Pinned: kept when notes are regenerated.",
};
export const NotFoundInTranscript: Story = {
  render: () => <Sample>{(lang) => <NoteBlock kind="missing" text={lang === "vi" ? "hỏi về các gói giá" : jot[3]!.me!} />}</Sample>,
};
export const WithSeveralCitations: Story = {
  render: () => (
    <Sample>
      {(lang) => (
        <NoteBlock
          kind="ai"
          text={pick(jot[2]! as { en: string; vi: string }, lang)}
          citations={[{ timeMs: citedMs(7) }, { timeMs: citedMs(9), visited: true }, { timeMs: citedMs(10), broken: true, text: "Tue" }]}
        />
      )}
    </Sample>
  ),
};
