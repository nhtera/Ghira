// SPDX-License-Identifier: Apache-2.0

import { clock, LINES, type Lang, SPEAKERS, speakerName } from "@/content/demo-data";
import { Avatar } from "./avatar";

/**
 * The cited line(s) shown under a note on narrow screens, where the
 * transcript panel is far below the notes.
 */
export function InlineQuote({ lines, lang }: { lines: readonly number[]; lang: Lang }) {
  return (
    <blockquote className="quote">
      {lines.map((i) => {
        const line = LINES[i];
        return (
          <div key={i}>
            <div className="quote-who">
              <Avatar s={line.s} lang={lang} small />
              <span style={{ color: `var(--s${SPEAKERS[line.s].slot})` }}>{speakerName(line.s, lang)}</span>
              <span className="mono">{clock(line.t)}</span>
            </div>
            <p lang={lang}>{line.text[lang]}</p>
          </div>
        );
      })}
    </blockquote>
  );
}
