// SPDX-License-Identifier: Apache-2.0
// A citation's quote in a bottom sheet, as in the design: the speaker chip
// ("Me 01:44"), the words in serif between curly quotes, and a big
// "Play from 01:44" button pinned under them.
import { formatClock } from "@ghi/i18n";
import { Avatar, PhoneButton, Sheet, type TranscriptSpeaker } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import type { Citation } from "../../bindings";

export type QuoteSheetProps = {
  citation: Citation | null;
  /** Who said it, when the citation names a speaker (colour, initial, label). */
  speaker?: TranscriptSpeaker;
  onClose: () => void;
  /** The meeting still has audio (a sensitive one has none). */
  canPlay?: boolean;
  onPlay: (ms: number) => void;
};

export function QuoteSheet({
  citation,
  speaker,
  onClose,
  canPlay: audio = true,
  onPlay,
}: QuoteSheetProps) {
  const { t } = useTranslation();
  const at = citation?.t0Ms ?? null;
  const time = at === null ? null : formatClock(at, { pad: true });
  const canPlay = audio && at !== null && !citation?.missing;
  const color =
    speaker && speaker.colorSlot > 0
      ? `var(--s${speaker.colorSlot})`
      : undefined;
  return (
    <Sheet
      open={citation !== null}
      onOpenChange={(open) => !open && onClose()}
      title={t("mobile.detail.quote.title")}
      closeLabel={t("mobile.sheet.close")}
      handleLabel={t("mobile.sheet.handle")}
      footer={
        canPlay && time ? (
          // Room under the button of its own: the sheet's safe-area padding is 0 where there is no home indicator, and the button must never touch the edge.
          <div className="pb-4">
            <PhoneButton
              variant="primary"
              icon="play_arrow"
              onClick={() => {
                onPlay(at);
                onClose();
              }}
            >
              {t("mobile.detail.playFrom", { time })}
            </PhoneButton>
          </div>
        ) : undefined
      }
    >
      {(speaker || time) && (
        <p className="text-ios-subhead m-0 mb-3 flex items-center gap-2">
          {speaker && (
            <>
              <Avatar
                kind={speaker.isMe ? "me" : "person"}
                name={speaker.label}
                initial={speaker.initial}
                colorSlot={speaker.colorSlot}
                size="md"
              />
              <b style={color ? { color } : undefined}>{speaker.label}</b>
            </>
          )}
          {time && (
            <span className="text-ios-footnote font-mono text-muted tabular-nums">
              {time}
            </span>
          )}
        </p>
      )}
      {citation?.missing ? (
        <p className="text-ios-body m-0 text-muted">
          {t("mobile.detail.quote.missing")}
        </p>
      ) : (
        <>
          <blockquote className="m-0 font-serif text-[1.0625rem] leading-[1.55] text-ink">
            {`“${citation?.quote ?? ""}”`}
          </blockquote>
          {citation?.stale && (
            <p className="text-ios-footnote mt-3 mb-0 text-muted">
              {t("mobile.detail.quote.stale")}
            </p>
          )}
        </>
      )}
    </Sheet>
  );
}
