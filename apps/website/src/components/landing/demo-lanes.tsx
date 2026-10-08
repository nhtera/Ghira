// SPDX-License-Identifier: Apache-2.0

import { type Lang, laneSegments, SPEAKERS, speakerName } from "@/content/demo-data";
import { Avatar } from "./avatar";

/** Who is speaking when: one bar track per speaker. Decorative (the transcript says it all). */
export function DemoLanes({ time, lang }: { time: number; lang: Lang }) {
  return (
    <div className="lanes" aria-hidden="true">
      {SPEAKERS.map((spk, s) => (
        <div className="lane" key={spk.slot}>
          <span className="lane-who">
            <Avatar s={s} lang={lang} small />
            {speakerName(s, lang)}
          </span>
          <div className="lane-track">
            {laneSegments(s, time).map((seg, k) => (
              <span className="seg" key={k} style={{ left: `${seg.left}%`, width: `${seg.width}%`, background: `var(--s${spk.slot})` }} />
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
