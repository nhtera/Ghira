// SPDX-License-Identifier: Apache-2.0

import { COPIED, type Lang, SAMPLE_LABELS, typedNotes } from "@/content/demo-data";
import { landing } from "@/content/landing";

/** "Your notes": the words typed during the call, with a caret on the one being typed. */
export function DemoNotesPanel({ time, lang }: { time: number; lang: Lang }) {
  return (
    <aside className="win-side" aria-label={landing.demo.notesLabel}>
      <p className="side-title">{COPIED[lang].yourNotes}</p>
      <ul className="my-notes">
        {typedNotes(time, lang).map((n, k) => (
          <li key={k} lang={lang}>
            {n.text}
            {n.typing ? <span className="caret" /> : null}
          </li>
        ))}
      </ul>
      <p className="side-hint">{SAMPLE_LABELS[lang].hint}</p>
    </aside>
  );
}
