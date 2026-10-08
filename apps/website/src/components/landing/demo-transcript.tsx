// SPDX-License-Identifier: Apache-2.0

import { type Lang, shownLines } from "@/content/demo-data";
import { TranscriptLine } from "./transcript-line";

/** The last six lines of the live transcript. Not announced: it changes several times a second. */
export function DemoTranscript({ time, lang }: { time: number; lang: Lang }) {
  return (
    <div className="tx" aria-live="off">
      {shownLines(time, lang)
        .slice(-6)
        .map((s) => (
          <TranscriptLine key={s.i} i={s.i} shown={s.shown} partial={s.partial} lang={lang} />
        ))}
    </div>
  );
}
