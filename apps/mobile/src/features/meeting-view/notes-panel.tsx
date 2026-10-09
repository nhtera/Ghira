// SPDX-License-Identifier: Apache-2.0
// M4 Notes tab: blocks by section with provenance (user / AI / AI-edited) and
// citation chips. A legend ("You wrote" / "Written by Ghira") explains the two
// looks once, the way the design does; AI sections carry the sparkle. The phone never writes notes, so a meeting without them says
// so and offers the cloud sheet.
import { formatClock } from "@ghi/i18n";
import { Button, Icon, ListRow, ListSection, NoteBlock } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import type { Citation, MeetingNotes } from "../../bindings";
import { openCloudSheet } from "./handoff";
import { useCloudOffered } from "../settings/use-cloud-offered";
import { groupBlocks, noteKind, toNoteCitation, uncoveredMarks } from "./notes-model";

export type NotesPanelProps = {
  meeting: string;
  cloudLocked: boolean;
  onSent: () => void;
  notes: MeetingNotes;
  visited: ReadonlySet<string>;
  onCite: (citation: Citation, key: string) => void;
  /** Play the audio from a moment (a mark); absent when there is no audio here. */
  onPlayAt?: (ms: number) => void;
};

export function NotesPanel({
  meeting,
  cloudLocked,
  onSent,
  notes,
  visited,
  onCite,
  onPlayAt,
}: NotesPanelProps) {
  const { t } = useTranslation();
  // The cloud entry only exists once the user offered cloud notes in Settings.
  const cloudOffered = useCloudOffered();
  const groups = groupBlocks(notes.blocks);
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
            {t(`mobile.detail.section.${g.key}`)}
            {g.blocks.some((b) => noteKind(b) === "ai") && (
              <Icon name="auto_awesome" size={14} className="size-3.5" />
            )}
          </h2>
          <div className="flex flex-col gap-4">
            {g.blocks.map((b) => (
              <NoteBlock
                key={b.gid}
                kind={noteKind(b)}
                showProvenance={noteKind(b) !== "ai"}
                text={b.text}
                citations={b.citations.map((c, i) =>
                  toNoteCitation(c, i, visited, `${b.gid}:${i}`),
                )}
                onCite={(i) => onCite(b.citations[i], `${b.gid}:${i}`)}
              />
            ))}
          </div>
        </section>
      ))}
      {marked}
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
