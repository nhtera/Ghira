// SPDX-License-Identifier: Apache-2.0

import type { ReactNode } from "react";
import { clock, COPIED, type Cited, LINES, type Lang, MY_NOTES, NOTES, SAMPLE_LABELS, speakerName } from "@/content/demo-data";
import { landing } from "@/content/landing";
import { Avatar } from "./avatar";
import { InlineQuote } from "./inline-quote";

/** The selected citation: which button (a line can be cited twice) and the lines it cites. */
export interface ActiveCite {
  id: string;
  lines: readonly number[];
}

export const DEFAULT_CITE: ActiveCite = { id: "summary:0", lines: NOTES.summary[0].c };

interface View {
  lang: Lang;
  active: ActiveCite;
  /** Show the cited lines under the active note (narrow screens). */
  quote: boolean;
  onCite: (cite: ActiveCite) => void;
}

/** A time chip: pressing it selects the transcript line(s) the sentence came from. */
function Cite({ id, c, view }: { id: string; c: readonly number[]; view: View }) {
  const time = clock(LINES[c[0]].t);
  return (
    <button
      type="button"
      className="cite"
      aria-pressed={view.active.id === id}
      aria-label={`${SAMPLE_LABELS[view.lang].showLine} ${time}`}
      onClick={() => view.onCite({ id, lines: c })}
    >
      {time}
    </button>
  );
}

function Quote({ id, view }: { id: string; view: View }) {
  return view.quote && view.active.id === id ? <InlineQuote lines={view.active.lines} lang={view.lang} /> : null;
}

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="notes-group">
      <h3>{title}</h3>
      {children}
    </div>
  );
}

function Bullets({ group, list, view }: { group: string; list: readonly Cited[]; view: View }) {
  return (
    <ul className="note-list bullets">
      {list.map((n, k) => {
        const id = `${group}:${k}`;
        return (
          <li className="note-text" key={id}>
            {n.text[view.lang]}
            <Cite id={id} c={n.c} view={view} />
            <Quote id={id} view={view} />
          </li>
        );
      })}
    </ul>
  );
}

/** What the user typed during the call, before Ghira filled it in. */
function TypedPad({ lang }: { lang: Lang }) {
  return (
    <div className="typed-pad">
      <ul>
        {MY_NOTES.map((m) => (
          <li key={m.t}>{m.text[lang]}</li>
        ))}
      </ul>
      <p>{landing.after.typedNote}</p>
    </div>
  );
}

/** The meeting's notes: summary, decisions, action items, the user's own notes filled in, open questions. */
export function NotesView({ mode, ...view }: View & { mode: "typed" | "full" }) {
  const { lang } = view;
  const words = COPIED[lang];
  return (
    <div className="notes" lang={lang}>
      {mode === "typed" ? (
        <TypedPad lang={lang} />
      ) : (
        <>
          <Group title={words.sections.summary}>
            <Bullets group="summary" list={NOTES.summary} view={view} />
          </Group>
          <Group title={words.sections.decisions}>
            <Bullets group="decisions" list={NOTES.decisions} view={view} />
          </Group>
          <Group title={words.sections.actionItems}>
            <ul className="note-list">
              {NOTES.actions.map((a, k) => {
                const id = `actions:${k}`;
                return (
                  <li className="action" key={id}>
                    <Avatar s={a.o} lang={lang} />
                    <div className="note-text">
                      {a.text[lang]}
                      <Cite id={id} c={a.c} view={view} />
                      <span className="action-meta">
                        {a.o < 0 ? words.unassigned : speakerName(a.o, lang)}. {a.due[lang]}
                      </span>
                      <Quote id={id} view={view} />
                    </div>
                  </li>
                );
              })}
            </ul>
          </Group>
          <Group title={words.sections.yourNotes}>
            <ul className="note-list">
              {NOTES.mine.map((m, k) => {
                const id = `mine:${k}`;
                return (
                  <li className="mine" key={id}>
                    <span className="typed">{MY_NOTES[m.k].text[lang]}</span>
                    {m.miss ? (
                      <p className="note-text missed">{words.notDiscussed}.</p>
                    ) : (
                      <>
                        <p className="note-text">
                          {m.text[lang]}
                          <Cite id={id} c={m.c} view={view} />
                        </p>
                        <Quote id={id} view={view} />
                      </>
                    )}
                  </li>
                );
              })}
            </ul>
          </Group>
          <Group title={words.sections.openQuestions}>
            <Bullets group="questions" list={NOTES.questions} view={view} />
          </Group>
        </>
      )}
    </div>
  );
}
