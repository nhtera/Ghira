// SPDX-License-Identifier: Apache-2.0

import { Fragment } from "react";
import { clock, COPIED, isIdentifying, LINES, type Lang, SPEAKERS, speakerName, wordsOf } from "@/content/demo-data";
import { Avatar } from "./avatar";

/**
 * One transcript line: time, avatar, speaker name and the words on screen.
 * `shown` words are visible; while `partial` the line is grey and its last
 * words fade in. Words the model is unsure of get a dotted underline once
 * the line is final.
 */
export function TranscriptLine({ i, shown, partial, lang, cited }: { i: number; shown: number; partial: boolean; lang: Lang; cited?: boolean }) {
  const line = LINES[i];
  const low = new Set((line.low?.[lang] ?? "").split(/\s+/).filter(Boolean));
  const identifying = isIdentifying({ i, shown, partial });
  const words = wordsOf(i, lang).slice(0, shown);
  return (
    <div className="line" data-i={i} data-state={partial ? "partial" : "final"} data-cited={cited ? "true" : undefined}>
      <span className="line-time">{clock(line.t)}</span>
      <Avatar s={identifying ? -1 : line.s} lang={lang} />
      <div>
        <div className="line-who">
          {identifying ? (
            <span className="identifying">{COPIED[lang].identifying}</span>
          ) : (
            <span style={{ color: `var(--s${SPEAKERS[line.s].slot})` }}>{speakerName(line.s, lang)}</span>
          )}
        </div>
        <p className="line-text" lang={lang}>
          {words.map((w, k) => {
            const fresh = partial && k >= shown - 2;
            const cls = [low.has(w) && !partial ? "w-low" : "", fresh ? "w-new" : ""].filter(Boolean).join(" ");
            return (
              // A fresh word is a new element each step, so its fade-in restarts.
              <Fragment key={fresh ? `${k}:${shown}` : k}>
                {k > 0 ? " " : null}
                {cls ? <span className={cls}>{w}</span> : w}
              </Fragment>
            );
          })}
        </p>
      </div>
    </div>
  );
}
