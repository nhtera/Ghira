// SPDX-License-Identifier: Apache-2.0
// "Discard the last N minutes" [RT-1], for off-record moments: lists what goes
// before anything is removed, then removes exactly that span (the cut it
// showed, however long the user took to confirm).
import { PhoneButton, Sheet } from "@ghi/ui";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { DiscardPreview } from "../../bindings";

const SHOWN = 3;

export type DiscardSheetProps = {
  /** The span to preview, in seconds; `null`: closed. */
  seconds: number | null;
  /** Sensitive mode: there is no audio to name in the summary. */
  sensitive: boolean;
  /** The live transcript trails the audio: text still pending for the span is dropped too. */
  behind?: boolean;
  preview: (seconds: number) => Promise<{ status: "ok"; data: DiscardPreview } | { status: "error"; error: string }>;
  /** Discards from the previewed cut on; resolves whether it worked. */
  onConfirm: (fromMs: number) => Promise<boolean>;
  onClose: () => void;
};

export function DiscardSheet({ seconds, sensitive, behind = false, preview, onConfirm, onClose }: DiscardSheetProps) {
  const { t } = useTranslation();
  // The preview with the span it was read for: another span shows nothing until its own arrives.
  const [loaded, setLoaded] = useState<{ seconds: number; data: DiscardPreview } | null>(null);
  const [busy, setBusy] = useState(false);
  const data = loaded && loaded.seconds === seconds ? loaded.data : null;

  useEffect(() => {
    if (seconds === null) return;
    let alive = true;
    void preview(seconds)
      .catch(() => null)
      .then((r) => {
        if (!alive) return;
        if (r?.status === "ok") setLoaded({ seconds, data: r.data });
        else onClose();
      });
    return () => {
      alive = false;
    };
    // The preview is for this request only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [seconds]);

  const confirm = async () => {
    if (!data || data.fromMs === null) return;
    setBusy(true);
    await onConfirm(data.fromMs);
    setBusy(false);
    onClose();
  };

  const minutes = Math.max(1, Math.round((seconds ?? 60) / 60));
  const items = data
    ? [
        data.lines.length ? t("mobile.record.discard.lines", { count: data.lines.length }) : "",
        data.notes.length ? t("mobile.record.discard.notes", { count: data.notes.length }) : "",
        data.marks ? t("mobile.record.discard.marks", { count: data.marks }) : "",
      ].filter(Boolean)
    : [];
  const shown = data ? [...data.lines, ...data.notes] : [];
  const summary = !data
    ? t("mobile.record.discard.loading")
    : items.length
      ? t(sensitive ? "mobile.record.discard.summaryTextOnly" : "mobile.record.discard.summaryItems", { items: items.join(", ") })
      : t(sensitive ? "mobile.record.discard.summaryNothing" : "mobile.record.discard.summaryAudioOnly");

  return (
    <Sheet
      open={seconds !== null}
      onOpenChange={(o) => !o && onClose()}
      title={t("mobile.record.discard.title", { count: minutes })}
      description={t("mobile.record.discard.body")}
      closeLabel={t("mobile.sheet.close")}
      handleLabel={t("mobile.sheet.handle")}
      footer={
        <>
          <PhoneButton disabled={!data || busy} onClick={() => void confirm()}>
            {t("mobile.record.discard.confirm")}
          </PhoneButton>
          <PhoneButton variant="secondary" onClick={onClose}>
            {t("mobile.common.cancel")}
          </PhoneButton>
        </>
      }
    >
      <div data-testid="discard-preview" className="flex flex-col gap-2">
        <p role="status" className="text-ios-body m-0 font-medium">
          {summary}
        </p>
        {behind && <p className="text-ios-footnote m-0 text-muted">{t("mobile.record.discard.behind")}</p>}
        {shown.length > 0 && (
          <ul className="text-ios-footnote m-0 list-none p-0 text-muted">
            {shown.slice(0, SHOWN).map((x, i) => (
              <li key={i} className="truncate">
                {x}
              </li>
            ))}
            {shown.length > SHOWN && <li>{`+${shown.length - SHOWN}`}</li>}
          </ul>
        )}
      </div>
    </Sheet>
  );
}
