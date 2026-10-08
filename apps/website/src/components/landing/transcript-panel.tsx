// SPDX-License-Identifier: Apache-2.0

import type { Ref } from "react";
import { LINES, type Lang, SAMPLE_LABELS } from "@/content/demo-data";
import { landing } from "@/content/landing";
import { TranscriptLine } from "./transcript-line";

/** The whole transcript beside the notes; the lines the selected note cites are highlighted. */
export function TranscriptPanel({ lang, cited, scrollerRef }: { lang: Lang; cited: readonly number[]; scrollerRef: Ref<HTMLDivElement> }) {
  return (
    <aside className="tx-panel" aria-label={landing.after.transcriptLabel}>
      <div className="tx-panel-head">
        <strong>{SAMPLE_LABELS[lang].transcript}</strong>
        <span>{SAMPLE_LABELS[lang].meta}</span>
      </div>
      <div className="tx-scroll" ref={scrollerRef}>
        {LINES.map((_, i) => (
          <TranscriptLine key={i} i={i} shown={99} partial={false} lang={lang} cited={cited.includes(i)} />
        ))}
      </div>
    </aside>
  );
}
