// SPDX-License-Identifier: Apache-2.0
// A citation's quote in a bottom sheet, with "Play from m:ss".
import { formatClock } from "@ghi/i18n";
import { Button, Sheet } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import type { Citation } from "../../bindings";

export type QuoteSheetProps = {
  citation: Citation | null;
  /** The speaker's label, when the citation names one. */
  speaker?: string;
  onClose: () => void;
  onPlay: (ms: number) => void;
};

export function QuoteSheet({
  citation,
  speaker,
  onClose,
  onPlay,
}: QuoteSheetProps) {
  const { t } = useTranslation();
  const at = citation?.t0Ms ?? null;
  const time = at === null ? null : formatClock(at, { pad: true });
  const canPlay = at !== null && !citation?.missing;
  return (
    <Sheet
      open={citation !== null}
      onOpenChange={(open) => !open && onClose()}
      title={t("mobile.detail.quote.title")}
      description={[speaker, time].filter(Boolean).join(" · ") || undefined}
      closeLabel={t("mobile.sheet.close")}
      handleLabel={t("mobile.sheet.handle")}
      footer={
        canPlay && time ? (
          <Button
            variant="primary"
            icon="play_arrow"
            className="min-h-ios-target"
            onClick={() => {
              onPlay(at);
              onClose();
            }}
          >
            {t("mobile.detail.playFrom", { time })}
          </Button>
        ) : undefined
      }
    >
      {citation?.missing ? (
        <p className="text-ios-body m-0 text-muted">
          {t("mobile.detail.quote.missing")}
        </p>
      ) : (
        <>
          <blockquote className="text-notes m-0 border-s-[3px] border-accent ps-3 font-serif">
            {citation?.quote}
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
