// SPDX-License-Identifier: Apache-2.0
// Notes tab (D6): Summary, template sections, Decisions, Action items, Open
// questions, Key quotes, Your notes, Topics. Every block is editable; "My
// notes only" hides what the app wrote and you did not touch.
import { formatClock } from "@ghi/i18n";
import { Icon, cn } from "@ghi/ui";
import { useId, useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingDetail } from "../../bindings";
import { useMeetingNotes } from "../../state/meeting-queries";
import { usePlayer } from "../../state/player";
import { templateName } from "../meeting/template-names";
import { ActionItems } from "./action-rows";
import { BlockRow } from "./block-row";
import { NotesContext } from "./notes-context";
import { hasMine, layoutNotes } from "./notes-model";
import { useNotesEdit } from "./use-notes-edit";
import { YourNotes } from "./your-notes";

function Section({ title, children }: { title: string; children: ReactNode }) {
  const id = useId();
  return (
    <section aria-labelledby={id} className="flex flex-col gap-3">
      <h2 id={id} className="text-heading m-0">
        {title}
      </h2>
      {children}
    </section>
  );
}

export function NotesTab({
  meeting,
  detail,
}: {
  meeting: string;
  detail: MeetingDetail;
}) {
  const { t, i18n } = useTranslation();
  const q = useMeetingNotes(meeting);
  const edit = useNotesEdit(meeting);
  const [onlyMine, setOnlyMine] = useState(false);
  const notes = q.data;
  const layout = useMemo(
    () => (notes ? layoutNotes(notes, onlyMine) : null),
    [notes, onlyMine],
  );
  const ctx = useMemo(
    () => ({
      meeting,
      speakers: detail.speakers,
      audioAvailable: detail.audioAvailable,
      edit,
    }),
    [meeting, detail.speakers, detail.audioAvailable, edit],
  );
  const vi = i18n.language === "vi";

  if (q.isPending)
    return (
      <div
        aria-busy="true"
        className="h-40 animate-pulse rounded-panel bg-sunk motion-reduce:animate-none"
      />
    );
  if (!notes || !layout)
    return (
      <p className="text-body text-muted">
        {t("system.commandFailed", { message: q.error?.message ?? "" })}
      </p>
    );

  const blocks = (list: typeof layout.tldr, label: string) =>
    list.map((b) => <BlockRow key={b.gid} block={b} label={label} />);

  const empty =
    !layout.tldr.length &&
    !layout.sections.length &&
    !layout.decisions.length &&
    !layout.actions.length &&
    !layout.questions.length &&
    !layout.quotes.length &&
    !layout.mine.length &&
    !layout.topics.length;

  return (
    <NotesContext.Provider value={ctx}>
      <div className="flex flex-col gap-6 pb-6">
        <div className="flex items-center">
          <button
            type="button"
            aria-pressed={onlyMine}
            onClick={() => setOnlyMine((v) => !v)}
            className={cn(
              "inline-flex h-7 items-center gap-1.5 rounded-ctl border px-2.5 text-[12.5px] font-medium",
              onlyMine
                ? "border-accent bg-accent-soft text-accent"
                : "border-ctl text-muted hover:text-ink",
            )}
          >
            <Icon name={onlyMine ? "check" : "person"} size={14} />
            {t("notes.onlyMine")}
          </button>
        </div>

        {onlyMine && !hasMine(notes) && (
          <div
            role="status"
            className="rounded-panel border border-line2 bg-surface p-4"
          >
            <b className="text-body block font-semibold">
              {t("detail.noMine.title")}
            </b>
            <p className="text-small m-0 mt-1 text-muted">
              {t("detail.noMine.body", {
                template: templateName(detail.template, t),
              })}
            </p>
          </div>
        )}

        {layout.tldr.length > 0 && (
          <Section title={t("notes.sections.summary")}>
            {blocks(layout.tldr, t("notes.blockLabel"))}
          </Section>
        )}
        {layout.sections.map(({ section, blocks: list }) => (
          <Section
            key={section.id}
            title={vi ? section.titleVi : section.titleEn}
          >
            {blocks(list, t("notes.blockLabel"))}
          </Section>
        ))}
        {layout.decisions.length > 0 && (
          <Section title={t("notes.sections.decisions")}>
            {blocks(layout.decisions, t("notes.blockLabel"))}
          </Section>
        )}
        {(layout.actions.length > 0 || !onlyMine) && (
          <Section title={t("notes.sections.actionItems")}>
            <ActionItems items={layout.actions} />
          </Section>
        )}
        {layout.questions.length > 0 && (
          <Section title={t("notes.sections.openQuestions")}>
            {blocks(layout.questions, t("notes.blockLabel"))}
          </Section>
        )}
        {layout.quotes.length > 0 && (
          <Section title={t("notes.sections.keyQuotes")}>
            {blocks(layout.quotes, t("notes.blockLabel"))}
          </Section>
        )}
        {(layout.mine.length > 0 || !onlyMine) && (
          <Section title={t("notes.sections.yourNotes")}>
            <YourNotes notes={layout.mine} />
          </Section>
        )}
        {layout.topics.length > 0 && (
          <Section title={t("detail.topics.title")}>
            <ul className="m-0 flex list-none flex-col gap-1 p-0">
              {layout.topics.map((b) => {
                const t0 = b.citations[0]?.t0Ms;
                return (
                  <li key={b.gid} className="flex items-center gap-2 text-body">
                    {t0 != null && detail.audioAvailable ? (
                      <button
                        type="button"
                        onClick={() => usePlayer.getState().seek(t0, true)}
                        aria-label={t("detail.playFrom", {
                          time: formatClock(t0),
                        })}
                        className="text-mono h-6 rounded-seg border border-line2 px-1.5 text-[12px] text-muted hover:text-accent"
                      >
                        {formatClock(t0)}
                      </button>
                    ) : null}
                    <span>{b.text}</span>
                  </li>
                );
              })}
            </ul>
          </Section>
        )}
        {empty && !onlyMine && (
          <p className="text-body m-0 text-muted">{t("notes.empty")}</p>
        )}
      </div>
    </NotesContext.Provider>
  );
}
