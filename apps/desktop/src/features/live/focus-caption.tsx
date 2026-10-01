// SPDX-License-Identifier: Apache-2.0
// Focus layout: the notepad is the page; the transcript shrinks to the latest
// words so the call stays in view without competing with the notes.
import { useTranslation } from "react-i18next";
import { TranscriptLine, wordsFromText } from "@ghi/ui";
import { useLive } from "../../state/live";
import { useSpeakerLabel, speakerNumber } from "../../state/speaker-label";

export function FocusCaption() {
  const { t } = useTranslation();
  const last = useLive((s) => s.lines.at(-1));
  const partial = useLive((s) => Object.values(s.partial).filter(Boolean).join(" "));
  const speaker = useLive((s) => (last?.speaker != null ? s.speakers[last.speaker] : undefined));
  const labelOf = useSpeakerLabel();
  if (!last && !partial) return null;
  return (
    <div data-testid="focus-caption" role="group" aria-label={t("nav.live")} aria-live="off" className="mx-auto w-full max-w-3xl rounded-row bg-surface2 p-1">
      {partial ? (
        <TranscriptLine startMs={last?.t1Ms ?? 0} speaker={null} words={wordsFromText(partial)} partial />
      ) : (
        last && (
          <TranscriptLine
            startMs={last.t0Ms ?? 0}
            speaker={speaker ? { label: labelOf(speaker), colorSlot: speaker.colorSlot, isMe: speaker.isMe, initial: speakerNumber(speaker) ?? undefined } : null}
            words={wordsFromText(last.text)}
          />
        )
      )}
    </div>
  );
}
