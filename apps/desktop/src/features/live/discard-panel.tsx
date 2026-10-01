// SPDX-License-Identifier: Apache-2.0
// "Discard the last N minutes" [RT-1]: an inline confirmation (never a modal
// mid-recording) that lists what goes before anything is removed.
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { InlineConfirm, useToast } from "@ghi/ui";
import type { DiscardPreview } from "../../bindings";
import { ipc } from "../../ipc";
import { noteLinesKey } from "./notepad";

const SHOWN = 3;

export function DiscardPanel({ meeting, seconds, onClose }: { meeting: string; seconds: number; onClose: () => void }) {
  const { t } = useTranslation();
  const { show } = useToast();
  const qc = useQueryClient();
  const [preview, setPreview] = useState<DiscardPreview | null>(null);

  useEffect(() => {
    let live = true;
    void ipc.commands.discardPreview(seconds).then((r) => {
      if (!live) return;
      if (r.status === "ok") setPreview(r.data);
      else {
        show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
        onClose();
      }
    });
    return () => {
      live = false;
    };
    // The preview is for this request only; toast/translation identity changes must not refetch.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [seconds]);

  const confirm = async () => {
    // Exactly the previewed span: cut at `fromMs`, even if time passed since the preview.
    const r = preview?.fromMs != null ? await ipc.commands.discardFrom(preview.fromMs) : await ipc.commands.discardLast(seconds);
    if (r.status === "error") show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
    else void qc.invalidateQueries({ queryKey: noteLinesKey(meeting) });
    onClose();
  };

  if (!preview) return null;
  const shown = [...preview.lines, ...preview.notes];
  const items: string[] = [
    preview.lines.length ? t("live.discard.lines", { count: preview.lines.length }) : "",
    preview.notes.length ? t("live.discard.notes", { count: preview.notes.length }) : "",
    preview.marks ? t("live.discard.marks", { count: preview.marks }) : "",
  ].filter(Boolean);
  const summary = items.length ? t("live.discard.summaryItems", { items: items.join(", ") }) : t("live.discard.summaryAudioOnly");
  return (
    <div data-testid="discard-panel" className="flex flex-col gap-2">
      <InlineConfirm question={t("live.discard.question", { count: Math.round(seconds / 60) })} confirmLabel={t("live.discard.confirm")} onConfirm={() => void confirm()} onCancel={onClose} />
      <div className="text-small rounded-row bg-surface2 px-3.5 py-2 text-muted">
        <p className="m-0 font-medium text-ink">{summary}</p>
        {shown.length > 0 && (
          <ul className="m-0 mt-1 list-none p-0">
            {shown.slice(0, SHOWN).map((x, i) => (
              <li key={i} className="truncate">
                {x}
              </li>
            ))}
            {shown.length > SHOWN && <li>{t("common.plusCount", { count: shown.length - SHOWN })}</li>}
          </ul>
        )}
      </div>
    </div>
  );
}
