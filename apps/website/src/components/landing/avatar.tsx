// SPDX-License-Identifier: Apache-2.0

import { type Lang, SPEAKERS, speakerName } from "@/content/demo-data";

/**
 * A speaker's colored initial. Decorative: the name is always written next to
 * it, so the color is never the only way to tell speakers apart. `s` < 0 is
 * a voice not yet identified (or a note with no owner).
 */
export function Avatar({ s, lang, small }: { s: number; lang: Lang; small?: boolean }) {
  const size = small ? " sm" : "";
  if (s < 0) {
    return (
      <span className={`avatar none${size}`} aria-hidden="true">
        ?
      </span>
    );
  }
  return (
    <span className={`avatar${size}`} style={{ background: `var(--s${SPEAKERS[s].slot})` }} aria-hidden="true">
      {speakerName(s, lang)[0]}
    </span>
  );
}
