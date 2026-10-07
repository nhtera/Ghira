// SPDX-License-Identifier: Apache-2.0
// M4 Notes tab: blocks by section with provenance (user / AI / AI-edited) and
// citation chips. A legend ("You wrote" / "Written by Ghira") explains the two
// looks once, the way the design does; AI sections carry the sparkle. A
// meeting without notes offers to write them on this phone (8 GB with the
// notes model), points to the model download (8 GB without it), and offers
// the cloud sheet when cloud notes are on.
import { Button, Icon, ListRow, ListSection, NoteBlock } from "@ghi/ui";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Citation, MeetingJob, MeetingNotes } from "../../bindings";
import { ipc } from "../../ipc";
import { useNotesModel } from "../models/use-notes-model";
import { useGo } from "../settings/go";
import { openCloudSheet } from "./handoff";
import { useCloudOffered } from "../settings/use-cloud-offered";
import { groupBlocks, noteKind, toNoteCitation } from "./notes-model";

export type NotesPanelProps = {
  meeting: string;
  cloudLocked: boolean;
  onSent: () => void;
  notes: MeetingNotes;
  visited: ReadonlySet<string>;
  onCite: (citation: Citation, key: string) => void;
  /** The meeting's running or waiting job, if any. */
  job: MeetingJob | null;
  /** There is a transcript to write notes from. */
  hasTranscript: boolean;
};

export function NotesPanel({
  meeting,
  cloudLocked,
  onSent,
  notes,
  visited,
  onCite,
  job,
  hasTranscript,
}: NotesPanelProps) {
  const { t } = useTranslation();
  const go = useGo();
  // The cloud entry only exists once the user offered cloud notes in Settings.
  const cloudOffered = useCloudOffered();
  const local = useNotesModel();
  const [failed, setFailed] = useState<string | null>(null);
  const groups = groupBlocks(notes.blocks);
  if (groups.length === 0) {
    // Nothing until this phone's notes model is known (no cloud copy flashing by).
    if (local.item === undefined && !local.error) return <div data-state="no-notes" className="min-h-40" />;
    const writing = job?.kind === "notes_final" || job?.kind === "notes_live";
    // Which empty state: being written, written after the transcript, can write
    // here, can after a download, or cloud only.
    const state = writing
      ? "writing"
      : job?.kind === "final_pass" && local.ready
        ? "after"
        : local.ready
          ? "local"
          : local.item
            ? "download"
            : "cloud";
    const write = async () => {
      setFailed(null);
      const r = await ipc.commands
        .writeNotes(meeting)
        .catch((e: unknown) => ({ status: "error" as const, error: String(e) }));
      if (r.status === "error") return setFailed(r.error);
      // The reloaded meeting carries the notes job: the "writing" state follows it.
      onSent();
    };
    return (
      <div
        data-state="no-notes"
        data-notes-state={state}
        className="flex flex-col items-center gap-3 px-6 py-10 text-center"
      >
        <Icon name={state === "cloud" ? "description" : "auto_awesome"} size={40} className="size-10 text-muted" />
        <h2 className="text-ios-title3 m-0">
          {t(`mobile.detail.notesEmpty.${state}.title`)}
        </h2>
        <p role={state === "writing" ? "status" : undefined} className="text-ios-subhead m-0 max-w-sm text-muted">
          {state === "writing" && job?.waitingForModels
            ? t("mobile.detail.notesEmpty.writing.waiting")
            : t(`mobile.detail.notesEmpty.${state}.body`)}
        </p>
        {state === "local" && (
          <Button
            variant="primary"
            icon="auto_awesome"
            className="min-h-ios-target px-5"
            disabled={!hasTranscript || job != null}
            onClick={() => void write()}
          >
            {t("mobile.detail.notesEmpty.local.action")}
          </Button>
        )}
        {(state === "download" || (state === "writing" && job?.waitingForModels && local.item && !local.ready)) && (
          <Button icon="download" className="min-h-ios-target px-5" onClick={() => go("/settings/models")}>
            {t("mobile.detail.notesEmpty.download.action")}
          </Button>
        )}
        {failed && (
          <p role="alert" className="text-ios-footnote m-0 text-warn">
            {t("system.commandFailed", { message: failed })}
          </p>
        )}
        {cloudOffered && state !== "writing" && (
          <Button
            variant={state === "cloud" ? "primary" : "secondary"}
            icon="cloud"
            className="min-h-ios-target px-5"
            onClick={() => openCloudSheet(meeting, cloudLocked, onSent)}
          >
            {t("mobile.detail.improveWithCloud")}
          </Button>
        )}
      </div>
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
