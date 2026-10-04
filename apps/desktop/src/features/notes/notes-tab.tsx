// SPDX-License-Identifier: Apache-2.0
// Notes tab (D6): Summary, template sections, Decisions, Action items, Open
// questions, Key quotes, Your notes, Topics. Every block is editable; "My
// notes only" hides what the app wrote and you did not touch.
import { APP_NAME, formatClock } from "@ghi/i18n";
import { Icon } from "@ghi/ui";
import { useId, useMemo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingDetail } from "../../bindings";
import { useMeetingNotes } from "../../state/meeting-queries";
import { usePlayer } from "../../state/player";
import { inProgress } from "../library/meeting-status";
import { templateName } from "../meeting/template-names";
import { ActionItems } from "./action-rows";
import { BlockRow } from "./block-row";
import { NotesContext } from "./notes-context";
import { hasMine, layoutNotes } from "./notes-model";
import { useNotesEdit } from "./use-notes-edit";
import { YourNotes } from "./your-notes";

/** `ai`: the app wrote this section, so its heading carries the sparkle the legend explains. */
function Section({ title, ai, children }: { title: string; ai?: boolean; children: ReactNode }) {
  const id = useId();
  return (
    <section aria-labelledby={id} className="mt-6 flex flex-col gap-2">
      <h2 id={id} className="m-0 flex items-center gap-1.5 font-sans text-[12px] font-semibold tracking-[0.07em] text-faint uppercase">
        {title}
        {ai && <Icon name="auto_awesome" size={13} />}
      </h2>
      {children}
    </section>
  );
}

function Bullets({ children }: { children: ReactNode[] }) {
  return <ul className="m-0 grid list-disc gap-1.5 pl-5 marker:text-faint">{children.map((c, i) => <li key={i}>{c}</li>)}</ul>;
}

export function NotesTab({
  meeting,
  detail,
  onlyMine = false,
}: {
  meeting: string;
  detail: MeetingDetail;
  /** "My notes only" (the switch is in the tab row): hides what the app wrote and you did not touch. */
  onlyMine?: boolean;
}) {
  const { t, i18n } = useTranslation();
  const q = useMeetingNotes(meeting);
  const edit = useNotesEdit(meeting);
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

  const blocks = (list: typeof layout.tldr, label: string) => (
    <Bullets>{list.map((b) => <BlockRow key={b.gid} block={b} label={label} />)}</Bullets>
  );

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
      <div className="flex flex-col pb-6">
        <div className="flex gap-[18px] font-sans text-[12px] text-muted">
          <span className="flex items-center gap-1.5">
            <span className="size-2.5 rounded-[2px] bg-ink" />
            {t("notes.youWrote")}
          </span>
          <span className="flex items-center gap-1.5 text-ai">
            <Icon name="auto_awesome" size={14} />
            {t("notes.writtenByApp", { app: APP_NAME })}
          </span>
        </div>

        {onlyMine && !hasMine(notes) && (
          <div
            role="status"
            className="mt-6 rounded-panel bg-surface2 p-4"
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
          <Section ai title={t("notes.sections.summary")}>
            {blocks(layout.tldr, t("notes.blockLabel"))}
          </Section>
        )}
        {layout.sections.map(({ section, blocks: list }) => (
          <Section
            ai
            key={section.id}
            title={vi ? section.titleVi : section.titleEn}
          >
            {blocks(list, t("notes.blockLabel"))}
          </Section>
        ))}
        {layout.decisions.length > 0 && (
          <Section ai title={t("notes.sections.decisions")}>
            {blocks(layout.decisions, t("notes.blockLabel"))}
          </Section>
        )}
        {(layout.actions.length > 0 || !onlyMine) && (
          <Section ai title={t("notes.sections.actionItems")}>
            <ActionItems items={layout.actions} />
          </Section>
        )}
        {layout.questions.length > 0 && (
          <Section ai title={t("notes.sections.openQuestions")}>
            {blocks(layout.questions, t("notes.blockLabel"))}
          </Section>
        )}
        {layout.quotes.length > 0 && (
          <Section ai title={t("notes.sections.keyQuotes")}>
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
                          time: formatClock(t0, { pad: true }),
                        })}
                        className="text-mono h-6 rounded-seg border border-line2 px-1.5 text-[12px] text-muted hover:text-accent"
                      >
                        {formatClock(t0, { pad: true })}
                      </button>
                    ) : null}
                    <span>{b.text}</span>
                  </li>
                );
              })}
            </ul>
          </Section>
        )}
        {empty && !onlyMine && !(inProgress(detail.status) || detail.job != null) && (
          <p className="text-body m-0 text-muted">{t("notes.empty")}</p>
        )}
      </div>
    </NotesContext.Provider>
  );
}
