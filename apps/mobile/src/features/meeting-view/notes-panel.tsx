// SPDX-License-Identifier: Apache-2.0
// M4 Notes tab: blocks by section with provenance (user / AI / AI-edited) and
// citation chips. A legend ("You wrote" / "Written by Ghira") explains the two
// looks once, the way the design does; AI sections carry the sparkle. The phone never writes notes, so a meeting without them says
// so and offers the cloud sheet.
import { formatClock } from "@ghi/i18n";
import { Button, Icon, ListRow, ListSection, NoteBlock } from "@ghi/ui";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { Citation, MeetingNotes, MeetingSpeaker } from "../../bindings";
import { openCloudSheet } from "./handoff";
import { useCloudOffered } from "../settings/use-cloud-offered";
import { Outline } from "./outline";
import { groupBlocks, noteKind, outlineInput, toNoteCitation, uncoveredMarks, type SectionKey } from "./notes-model";

export type NotesPanelProps = {
  meeting: string;
  cloudLocked: boolean;
  onSent: () => void;
  notes: MeetingNotes;
  visited: ReadonlySet<string>;
  onCite: (citation: Citation, key: string) => void;
  /** Play the audio from a moment (a mark); absent when there is no audio here. */
  onPlayAt?: (ms: number) => void;
  /** The meeting's title and speakers, for the outline. */
  title?: string;
  speakers?: MeetingSpeaker[];
};

export function NotesPanel({
  meeting,
  cloudLocked,
  onSent,
  notes,
  visited,
  onCite,
  onPlayAt,
  title = "",
  speakers = [],
}: NotesPanelProps) {
  const { t, i18n } = useTranslation();
  // The cloud entry only exists once the user offered cloud notes in Settings.
  const cloudOffered = useCloudOffered();
  const vi = i18n.language === "vi";
  const groups = groupBlocks(notes.blocks, notes.sections, vi);
  const outline = useMemo(
    () =>
      outlineInput(notes, {
        title,
        vi,
        titles: {
          summary: t("notes.sections.summary"),
          decisions: t("notes.sections.decisions"),
          proposed: t("mindmap.sections.proposed"),
          actions: t("notes.sections.actionItems"),
          questions: t("notes.sections.openQuestions"),
          topics: t("detail.topics.title"),
          marked: t("mindmap.sections.marked"),
          other: t("mindmap.sections.other"),
        },
        markLabel: (tag) =>
          t(
            `notes.tags.${tag === "decision" || tag === "action" || tag === "question" ? tag : "star"}`,
          ),
      }),
    [notes, title, vi, t],
  );
  const missed = uncoveredMarks(notes.marks);
  const marked = missed.length > 0 && (
    <section aria-labelledby="sec-marked" className="px-4 pt-4">
      <h2
        id="sec-marked"
        className="text-ios-footnote m-0 mb-2 font-semibold tracking-[0.07em] text-muted uppercase"
      >
        {t("notes.sections.marked")}
      </h2>
      <ul
        data-testid="marked-moments"
        className="m-0 flex list-none flex-col gap-2 p-0"
      >
        {missed.map((m, i) => {
          const time = formatClock(m.tMs ?? 0, { pad: true });
          const tag = t(
            `notes.tags.${m.tag === "decision" || m.tag === "action" || m.tag === "question" ? m.tag : "star"}`,
          );
          return (
            <li key={i} className="flex flex-col gap-0.5">
              <span className="text-ios-footnote inline-flex items-center gap-1.5 text-muted">
                <Icon name="star" size={14} className="size-3.5 text-warn" />
                {tag}
                {onPlayAt ? (
                  <button
                    type="button"
                    onClick={() => onPlayAt(m.tMs ?? 0)}
                    aria-label={t("mobile.detail.playFrom", { time })}
                    className="min-h-ios-target font-mono text-accent tabular-nums"
                  >
                    {time}
                  </button>
                ) : (
                  <span className="font-mono tabular-nums">{time}</span>
                )}
              </span>
              {m.text && (
                <span className="text-ios-body font-serif">{m.text}</span>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
  if (groups.length === 0) {
    return (
      <>
      <div
        data-state="no-notes"
        className="flex flex-col items-center gap-3 px-6 py-10 text-center"
      >
        <Icon name="description" size={40} className="size-10 text-muted" />
        <h2 className="text-ios-title3 m-0">
          {t("mobile.detail.notesEmpty.title")}
        </h2>
        <p className="text-ios-subhead m-0 max-w-sm text-muted">
          {t("mobile.detail.notesEmpty.body")}
        </p>
        {cloudOffered && (
          <Button
            variant="primary"
            icon="cloud"
            className="min-h-ios-target px-5"
            onClick={() => openCloudSheet(meeting, cloudLocked, onSent)}
          >
            {t("mobile.detail.improveWithCloud")}
          </Button>
        )}
      </div>
      {marked}
      </>
    );
  }
  return (
    <div className="pb-4">
      <p className="text-ios-footnote m-0 flex flex-wrap items-center gap-x-4 gap-y-1 px-4 pt-1 text-muted">
        <span className="inline-flex items-center gap-1.5">
          <span aria-hidden="true" className="size-3 rounded-[3px] bg-ink" />
          {t("notes.youWrote")}
        </span>
        <span className="inline-flex items-center gap-1.5">
          <Icon name="auto_awesome" size={16} className="size-4" />
          {t("mobile.detail.writtenBy")}
        </span>
        {notes.linked > 0 && (
          <span data-testid="sources-linked" className="ms-auto">
            {t("notes.sourcesLinked", { count: notes.linked })}
          </span>
        )}
      </p>
      {groups.map((g) => (
        <section
          key={g.key}
          aria-labelledby={`sec-${g.key}`}
          className="px-4 pt-4"
        >
          <h2
            id={`sec-${g.key}`}
            className="text-ios-footnote m-0 mb-2 flex items-center gap-1.5 font-semibold tracking-[0.07em] text-muted uppercase"
          >
            {g.title ?? t(`mobile.detail.section.${g.key as SectionKey}`)}
            {g.blocks.some((b) => noteKind(b) === "ai") && (
              <Icon name="auto_awesome" size={14} className="size-3.5" />
            )}
          </h2>
          <div className="flex flex-col gap-4">
            {g.blocks.map((b, bi) => (
              <div key={b.gid} className="flex flex-col gap-1.5">
                {b.kind === "proposal" && (
                  <span
                    data-testid="proposed-chip"
                    className="text-ios-caption1 inline-flex h-5 items-center self-start rounded-[5px] border border-dashed border-line2 px-1.5 font-semibold text-muted"
                  >
                    {t("notes.proposed")}
                  </span>
                )}
                <NoteBlock
                  kind={noteKind(b)}
                  showProvenance={noteKind(b) !== "ai"}
                  text={b.text}
                  citations={b.citations.map((c, i) =>
                    toNoteCitation(c, i, visited, `${b.gid}:${i}`),
                  )}
                  onCite={(i) => onCite(b.citations[i], `${b.gid}:${i}`)}
                />
                {b.kind === "proposal" &&
                  g.blocks[bi + 1]?.kind !== "proposal" && (
                    <p
                      data-testid="proposal-footnote"
                      className="text-ios-footnote m-0 text-muted"
                    >
                      {t("notes.proposalFootnote")}
                    </p>
                  )}
              </div>
            ))}
          </div>
        </section>
      ))}
      {marked}
      <Outline input={outline} speakers={speakers} onPlayAt={onPlayAt} />
      {cloudOffered && (
        <ListSection className="mt-6">
          <ListRow
            icon="cloud"
            title={t("mobile.detail.improveWithCloud")}
            onPress={() => openCloudSheet(meeting, cloudLocked, onSent)}
            chevron
          />
        </ListSection>
      )}
    </div>
  );
}
