// SPDX-License-Identifier: Apache-2.0
// Import (D10): choose or drop files, review them (problems inline), pick the
// language and channel handling, then watch the queue. Dropping files on the
// window is handled by the core (it stages them and navigates here); the
// webview only shows the drop affordance.
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { formatBytes, formatClock, type Locale } from "@ghi/i18n";
import { Button, Icon, Segmented, cn, useToast, usePlatform, type IconName } from "@ghi/ui";
import type { ImportSource, StagedFile } from "../../bindings";
import { ipc } from "../../ipc";
import { Page } from "../../shell/page";
import { activeCount, doneCount, importChoice, isActive, isImportable, type Language, type QueueItem } from "./import-model";
import { useImportStore } from "./import-store";

const SOURCE_ICON: Record<ImportSource, IconName> = {
  plaud: "mic",
  zoom: "videocam",
  teams: "videocam",
  voiceMemos: "graphic_eq",
  other: "music_note",
};

/** True while files are dragged over the window (shows the drop target; the core handles the drop). */
function useFileDragOver() {
  const [over, setOver] = useState(false);
  useEffect(() => {
    let depth = 0;
    const hasFiles = (e: DragEvent) => !!e.dataTransfer?.types?.includes("Files");
    const enter = (e: DragEvent) => hasFiles(e) && (depth++, setOver(true));
    const leave = (e: DragEvent) => hasFiles(e) && (depth = Math.max(0, depth - 1)) === 0 && setOver(false);
    const end = () => ((depth = 0), setOver(false));
    window.addEventListener("dragenter", enter);
    window.addEventListener("dragleave", leave);
    window.addEventListener("drop", end);
    window.addEventListener("dragend", end);
    return () => {
      window.removeEventListener("dragenter", enter);
      window.removeEventListener("dragleave", leave);
      window.removeEventListener("drop", end);
      window.removeEventListener("dragend", end);
    };
  }, []);
  return over;
}

export function ImportScreen() {
  const { t, i18n } = useTranslation();
  const platform = usePlatform();
  const navigate = useNavigate();
  const { show } = useToast();
  // The root listens for drops; read what the core kept for us (a cold launch drops before any listener exists).
  useEffect(() => {
    void ipc.commands.takeDroppedFiles().then((files) => useImportStore.getState().addStaged(files));
  }, []);
  const staged = useImportStore((s) => s.staged);
  const queue = useImportStore((s) => s.queue);
  const over = useFileDragOver();
  const [language, setLanguage] = useState<Language>("auto");
  const [split, setSplit] = useState(true);
  const [busy, setBusy] = useState(false);
  const importable = staged.filter(isImportable);
  const stereo = staged.some((f) => f.channels >= 2);
  const items = Object.values(queue);
  const active = activeCount(queue);

  // One toast when the last running file is done.
  const wasActive = useRef(0);
  useEffect(() => {
    if (wasActive.current > 0 && active === 0 && doneCount(queue) > 0)
      show({
        tone: "success",
        title: t("import.done", { count: doneCount(queue) }),
      });
    wasActive.current = active;
  }, [active, queue, show, t]);

  const fail = (message: string) => show({ tone: "warning", title: t("system.commandFailed", { message }) });
  const choose = async () => {
    const r = await ipc.commands.pickImportFiles(t("import.chooseFiles"));
    if (r.status === "error") return fail(r.error);
    useImportStore.getState().addStaged(r.data);
  };
  const remove = (id: string) => {
    useImportStore.getState().removeStaged([id]);
    void ipc.commands.unstageFiles([id]);
  };
  const start = async () => {
    setBusy(true);
    try {
      const r = await ipc.commands.startImport(
        importable.map((f) => f.id),
        importChoice(language, split, importable),
      );
      if (r.status === "error") return fail(r.error);
      const st = useImportStore.getState();
      st.dispatch({
        type: "start",
        files: importable.map((f) => ({ id: f.id, name: f.name })),
      });
      st.removeStaged(importable.map((f) => f.id));
    } finally {
      setBusy(false);
    }
  };
  const openMeeting = (id: string) => void navigate({ to: "/meetings/$id/$tab", params: { id, tab: "notes" } });

  return (
    <Page
      title={t("nav.import")}
      subtitle={t("import.subtitle", { context: platform })}
      className="flex flex-col gap-5"
      actions={
        <Button variant="primary" icon="upload_file" onClick={() => void choose()}>
          {t("import.chooseFiles")}
        </Button>
      }
    >
      {staged.length === 0 && items.length === 0 && (
        <div className="flex flex-col items-center gap-2 rounded-panel border-[1.5px] border-dashed border-line2 px-6 py-12 text-center">
          <Icon name="upload_file" size={36} className="text-accent" />
          <b className="text-heading font-semibold">{t("import.drop.title")}</b>
          <span className="text-small text-muted">{t("import.formats")}</span>
          <span className="text-small text-faint">{t("import.dropHint", { context: platform })}</span>
        </div>
      )}

      {staged.length > 0 && (
        <section aria-labelledby="import-staged" className="flex flex-col gap-2">
          <h2 id="import-staged" className="text-small m-0 font-semibold text-muted">
            {t("import.staged.title")}
          </h2>
          <ul className="m-0 flex list-none flex-col gap-1.5 p-0">
            {staged.map((f) => (
              <StagedRow key={f.id} file={f} locale={(i18n.language === "vi" ? "vi" : "en") as Locale} onRemove={() => remove(f.id)} onOpen={openMeeting} />
            ))}
          </ul>
          <div className="mt-1 flex flex-col gap-3 rounded-panel border border-line bg-surface p-3.5" role="group" aria-label={t("import.options.title")}>
            <div className="flex flex-wrap items-center gap-3">
              <span className="text-small font-semibold text-muted">{t("import.options.language")}</span>
              <Segmented
                label={t("import.options.language")}
                value={language}
                onChange={setLanguage}
                options={[
                  { value: "auto", label: t("import.options.languages.auto") },
                  { value: "en", label: t("import.options.languages.english") },
                  {
                    value: "vi",
                    label: t("import.options.languages.vietnamese"),
                  },
                ]}
              />
            </div>
            {stereo && (
              <button
                type="button"
                role="checkbox"
                aria-checked={split}
                onClick={() => setSplit(!split)}
                className="flex items-start gap-2 rounded-seg text-left"
              >
                <Icon name={split ? "check_box" : "check_box_outline_blank"} size={20} className={split ? "text-accent" : "text-muted"} />
                <span className="flex flex-col">
                  <span className="text-body font-medium">{t("import.options.stereo")}</span>
                  <span className="text-small text-muted">{t("import.options.stereoHint")}</span>
                  <span className="text-small text-muted">{t("import.options.stereoSides")}</span>
                </span>
              </button>
            )}
            <div className="flex justify-end">
              <Button variant="primary" disabled={busy || importable.length === 0} onClick={() => void start()}>
                {t("import.importFiles", { count: importable.length })}
              </Button>
            </div>
          </div>
        </section>
      )}

      {items.length > 0 && (
        <section aria-labelledby="import-queue" className="flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <h2 id="import-queue" className="text-small m-0 font-semibold text-muted">
              {t("import.queue.title")}
            </h2>
            {active > 0 && (
              <span role="status" className="text-small rounded-full bg-accent-soft px-2 py-0.5 font-semibold text-accent">
                {t("import.queue.importing", { count: active })}
              </span>
            )}
            {active === 0 && (
              <button
                type="button"
                onClick={() => useImportStore.getState().dispatch({ type: "clear" })}
                className="text-small ml-auto rounded-seg px-2 font-semibold text-accent hover:bg-sunk"
              >
                {t("import.queue.clear")}
              </button>
            )}
          </div>
          {active > 0 && <p className="text-small m-0 text-muted">{t("import.keepWorking")}</p>}
          <ul className="m-0 flex list-none flex-col gap-1.5 p-0">
            {items.map((i) => (
              <QueueRow key={i.id} item={i} onOpen={openMeeting} />
            ))}
          </ul>
        </section>
      )}
      {over && (
        <div
          role="presentation"
          className="pointer-events-none fixed inset-3 z-40 grid place-items-center rounded-dialog border-2 border-dashed border-accent bg-accent-soft/80"
        >
          <div className="flex flex-col items-center gap-1 text-center">
            <Icon name="upload_file" size={40} className="text-accent" />
            <b className="text-heading font-semibold text-accent">{t("import.dropOverlay.title")}</b>
            <span className="text-body text-muted">{t("import.dropOverlay.body")}</span>
          </div>
        </div>
      )}
    </Page>
  );
}

function Tag({ tone, children }: { tone: "bad" | "warn"; children: ReactNode }) {
  return <span className={cn("text-small", tone === "bad" ? "font-semibold text-rec-ink" : "text-warn")}>{children}</span>;
}

function StagedRow({ file: f, locale, onRemove, onOpen }: { file: StagedFile; locale: Locale; onRemove: () => void; onOpen: (meeting: string) => void }) {
  const { t } = useTranslation();
  const blocked = !isImportable(f);
  const meta = [
    t(`import.sources.${f.source}`),
    f.durationMs != null ? formatClock(f.durationMs) : null,
    f.sizeBytes != null ? formatBytes(f.sizeBytes, locale) : null,
    f.channels >= 2 ? t("import.stereo") : null,
  ]
    .filter(Boolean)
    .join(t("common.metaSep"));
  return (
    <li
      data-blocked={blocked ? "true" : undefined}
      className={cn("flex flex-wrap items-center gap-3 rounded-row border border-line bg-surface px-3 py-2.5", blocked && "bg-surface2")}
    >
      <Icon name={SOURCE_ICON[f.source]} size={22} className="flex-none text-muted" />
      <div className="min-w-0 flex-1">
        <span className={cn("block truncate text-[13.5px] font-semibold", blocked && "text-muted")}>{f.name}</span>
        <span className="text-small block text-muted">{meta}</span>
        {f.problems.includes("unsupported") && <Tag tone="bad">{t("import.problems.unsupported")}</Tag>}
        {f.problems.includes("empty") && <Tag tone="bad">{t("import.problems.empty")}</Tag>}
        {f.problems.includes("veryLong") && <Tag tone="warn">{t("import.problems.veryLong")}</Tag>}
        {f.problems.includes("duplicate") && f.duplicateOf && (
          <span className="text-small flex flex-wrap items-center gap-1.5">
            <Tag tone="warn">{t("import.problems.duplicate", { title: f.duplicateOf.title })}</Tag>
            <button type="button" onClick={() => onOpen(f.duplicateOf!.meeting)} className="font-semibold text-accent hover:underline">
              {t("common.open")}
            </button>
          </span>
        )}
      </div>
      {blocked && !f.problems.includes("duplicate") && <span className="text-small text-muted">{t("import.wontImport")}</span>}
      <Button variant="ghost" size="sm" icon="close" aria-label={t("import.removeFile", { name: f.name })} title={t("common.remove")} onClick={onRemove} />
    </li>
  );
}

function QueueRow({ item, onOpen }: { item: QueueItem; onOpen: (meeting: string) => void }) {
  const { t } = useTranslation();
  const pct = Math.round((item.progress ?? 0) * 100);
  const status =
    item.state === "queued"
      ? t("import.queue.waiting")
      : item.state === "decoding"
        ? t("import.queue.decoding", { percent: pct })
        : item.state === "done"
          ? t("import.queue.ready")
          : item.state === "cancelled"
            ? t("import.queue.cancelled")
            : t("import.queue.failed", { message: item.error ?? "" });
  return (
    <li data-state={item.state} className="flex flex-wrap items-center gap-3 rounded-row border border-line bg-surface px-3 py-2.5">
      <Icon
        name={item.state === "done" ? "check_circle" : item.state === "failed" ? "error" : item.state === "cancelled" ? "block" : "progress_activity"}
        size={20}
        className={cn("flex-none", item.state === "done" ? "text-accent" : "text-muted")}
      />
      <div className="min-w-0 flex-1">
        <span className="block truncate text-[13.5px] font-semibold">{item.name}</span>
        <span className={cn("text-small block", item.state === "failed" ? "text-rec-ink" : "text-muted")}>{status}</span>
        {item.state === "decoding" && (
          <div
            role="progressbar"
            aria-label={item.name}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct}
            className="mt-1 h-1 overflow-hidden rounded-full bg-sunk"
          >
            <div className="h-full bg-accent transition-[width]" style={{ width: `${pct}%` }} />
          </div>
        )}
      </div>
      {item.state === "done" && item.meeting && (
        <Button size="sm" onClick={() => onOpen(item.meeting!)}>
          {t("import.queue.openNotes")}
        </Button>
      )}
      {isActive(item) && (
        <Button variant="ghost" size="sm" aria-label={t("import.queue.cancel", { name: item.name })} onClick={() => void ipc.commands.cancelImport(item.id)}>
          {t("common.cancel")}
        </Button>
      )}
    </li>
  );
}
