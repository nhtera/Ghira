// SPDX-License-Identifier: Apache-2.0
// Import (D10): choose or drop files, review them (problems inline), pick the
// language and channel handling, then watch the queue. Dropping files on the
// window is handled by the core (it stages them and navigates here); the
// webview only shows the drop affordance.
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { APP_NAME, formatBytes, formatClock, formatDate, type Locale } from "@ghi/i18n";
import { Button, Icon, Segmented, cn, useToast, usePlatform, type IconName } from "@ghi/ui";
import type { ImportSource, StagedFile } from "../../bindings";
import { ipc } from "../../ipc";
import { Page } from "../../shell/page";
import { IMPORT_ERROR_CODES, activeCount, doneCount, importChoice, importableFiles, isActive, isImportable, unitsOf, type Language, type QueueItem, type Unit } from "./import-model";
import { useImportStore } from "./import-store";

const SOURCE_ICON: Record<ImportSource, IconName> = {
  plaud: "mic",
  zoom: "videocam",
  teams: "videocam",
  meet: "videocam",
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
  const units = unitsOf(staged);
  // A group's tracks go together; a track left alone goes as a file.
  const importable = units.flatMap(importableFiles);
  const stereo = staged.some((f) => f.channels >= 2 && !f.group);
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
    // Removing the last track brings the mixed recording back.
    void ipc.commands.unstageFiles([id]).then((changed) => useImportStore.getState().updateStaged(changed));
  };
  const separate = async (group: string) => {
    const r = await ipc.commands.importTracksSeparately(group);
    if (r.status === "error") return fail(r.error);
    useImportStore.getState().updateStaged(r.data);
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
        files: units
          .filter((u) => importableFiles(u).length > 0)
          .map((u) => ({ id: u.group && importableFiles(u).length >= 2 ? u.id : importableFiles(u)[0]!.id, name: unitName(u, t) })),
      });
      st.removeStaged(importable.map((f) => f.id));
    } finally {
      setBusy(false);
    }
  };
  const openMeeting = (id: string) => void navigate({ to: "/meetings/$id/$tab", params: { id, tab: "notes" } });

  return (
    <Page title={t("nav.import")} subtitle={t("import.subtitle", { context: platform })} className="flex max-w-[912px] flex-col gap-5">
      {/* The whole zone opens the chooser; the button is the keyboard way in. */}
      <div
        onClick={() => void choose()}
        className="flex cursor-pointer flex-col items-center gap-2 rounded-dialog border-2 border-dashed border-line2 bg-surface2 px-5 py-[30px] text-center hover:border-accent"
      >
        <Icon name="upload_file" size={36} className="text-accent" />
        <b className="text-[16px] font-semibold">{t("import.drop.title")}</b>
        <span className="text-[12.5px] text-muted">{t("import.formats")}</span>
        <Button
          variant="primary"
          size="lg"
          className="mt-1.5"
          onClick={(e) => {
            e.stopPropagation();
            void choose();
          }}
        >
          {t("import.chooseFiles")}
        </Button>
        <span className="text-[12px] text-faint">{t("import.dropHint", { context: platform })}</span>
      </div>

      {staged.length > 0 && (
        <section aria-labelledby="import-staged" className="flex flex-col gap-2">
          <div className="flex items-baseline gap-2">
            <h2 id="import-staged" className="m-0 text-[14px] font-semibold">
              {t("import.staged.title")}
            </h2>
            <span className="text-[12.5px] text-muted">
              {importable.length} / {staged.length}
            </span>
          </div>
          <ul className="m-0 flex list-none flex-col overflow-hidden rounded-panel border border-line p-0">
            {units.map((u) =>
              u.group ? (
                <GroupRow key={u.id} unit={u} locale={(i18n.language === "vi" ? "vi" : "en") as Locale} onRemove={remove} onSeparate={() => void separate(u.id)} onOpen={openMeeting} />
              ) : (
                <StagedRow key={u.id} file={u.files[0]!} locale={(i18n.language === "vi" ? "vi" : "en") as Locale} onRemove={() => remove(u.files[0]!.id)} onOpen={openMeeting} />
              ),
            )}
          </ul>
          <div className="mt-1 flex flex-col gap-3 rounded-panel bg-surface2 px-4 py-3.5" role="group" aria-label={t("import.options.title")}>
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
          </div>
          <div className="mt-1 flex items-center gap-3">
            <Button variant="primary" size="lg" className="h-[38px] px-[18px] text-[13.5px]" disabled={busy || importable.length === 0} onClick={() => void start()}>
              {t("import.importFiles", { count: importable.length })}
            </Button>
          </div>
        </section>
      )}

      <section aria-labelledby="import-queue" className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <h2 id="import-queue" className="m-0 text-[14px] font-semibold">
            {t("import.queue.title")}
          </h2>
          {active > 0 && (
            <span role="status" className="text-small rounded-full bg-accent-soft px-2 py-0.5 font-semibold text-accent">
              {t("import.queue.importing", { count: active })}
            </span>
          )}
          {items.length > 0 && active === 0 && (
            <button
              type="button"
              onClick={() => useImportStore.getState().dispatch({ type: "clear" })}
              className="text-small ml-auto rounded-seg px-2 font-semibold text-accent hover:bg-sunk"
            >
              {t("import.queue.clear")}
            </button>
          )}
        </div>
        {items.length === 0 ? (
          <p className="m-0 flex items-center gap-2 rounded-panel bg-surface2 p-4 text-[13px] text-muted">
            <Icon name="inbox" size={20} className="flex-none" />
            {t("import.queue.empty")}
          </p>
        ) : (
          <>
            {active > 0 && <p className="text-small m-0 text-muted">{t("import.keepWorking")}</p>}
            <ul className="m-0 flex list-none flex-col gap-2 p-0">
              {items.map((i) => (
                <QueueRow key={i.id} item={i} onOpen={openMeeting} />
              ))}
            </ul>
          </>
        )}
      </section>
      {over && (
        <div role="presentation" className="pointer-events-none fixed inset-x-0 top-9 bottom-0 z-40 flex bg-[var(--scrim)] p-5">
          <div className="flex flex-1 flex-col items-center justify-center gap-2.5 rounded-[16px] border-[2.5px] border-dashed border-accent bg-surface text-center">
            <Icon name="download" size={48} className="text-accent" />
            <b className="text-[20px] font-semibold">{t("import.dropOverlay.title")}</b>
            <span className="text-[13.5px] text-muted">{t("import.dropOverlay.body")}</span>
          </div>
        </div>
      )}
    </Page>
  );
}

/** Keys of the 14d import copy, not in the typed locale until PENDING-s7b.json is merged. */
const KEYS = { superseded: "import.problems.superseded", tooManyTracks: "import.problems.tooManyTracks", separately: "import.group.separately" } as const;

/** A word that isn't in the typed locale until the 14d import copy (apps/desktop/PENDING-s7b.json) is merged. */
const word = (t: TFunction, key: string): string => (t as unknown as (k: string) => string)(key);

/** The queue's name for a unit: the file's name, or the recording's title. */
function unitName(u: Unit, t: TFunction): string {
  const f = u.files[0]!;
  return u.group ? (f.title ?? t("import.group.title", { count: u.files.length })) : f.name;
}

function Tag({ tone, children }: { tone: "bad" | "warn"; children: ReactNode }) {
  return (
    <span className={cn("flex items-start gap-1.5 text-[12.5px] leading-[1.45]", tone === "bad" ? "text-rec-ink" : "text-warn")}>
      <Icon name={tone === "bad" ? "error" : "warning"} size={16} className="mt-px flex-none" />
      <span>{children}</span>
    </span>
  );
}

function StagedRow({ file: f, locale, onRemove, onOpen }: { file: StagedFile; locale: Locale; onRemove: () => void; onOpen: (meeting: string) => void }) {
  const { t } = useTranslation();
  const blocked = !isImportable(f);
  const meta = [
    f.title,
    f.startedAt != null ? formatDate(f.startedAt, locale) : null,
    f.sizeBytes != null ? formatBytes(f.sizeBytes, locale) : null,
    f.durationMs != null ? formatClock(f.durationMs) : null,
    f.channels >= 2 ? t("import.stereo") : null,
  ]
    .filter(Boolean)
    .join(t("common.metaSep"));
  return (
    <li
      data-blocked={blocked ? "true" : undefined}
      className={cn("flex flex-wrap items-start gap-x-3 gap-y-1 border-t border-line px-3.5 py-3 first:border-t-0", blocked && "bg-surface2")}
    >
      <Icon name={SOURCE_ICON[f.source]} size={22} className="mt-px flex-none text-muted" />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <span className={cn("block truncate text-[13.5px] font-semibold", blocked && "text-muted")}>{f.name}</span>
        <span className="flex flex-wrap items-center gap-2 text-[12px] text-muted">
          <span className="inline-flex h-[22px] items-center rounded-full bg-sunk px-2 font-semibold text-ink">{t(`import.sources.${f.source}`)}</span>
          {meta}
        </span>
        {f.problems.includes("unsupported") && <Tag tone="bad">{t("import.problems.unsupported")}</Tag>}
        {f.problems.includes("empty") && <Tag tone="bad">{t("import.problems.empty")}</Tag>}
        {f.problems.includes("veryLong") && <Tag tone="warn">{t("import.problems.veryLong")}</Tag>}
        {f.problems.includes("superseded") && <Tag tone="warn">{word(t, KEYS.superseded)}</Tag>}
        {f.problems.includes("duplicate") && f.duplicateOf && (
          <span className="flex flex-wrap items-center gap-1.5">
            <Tag tone="warn">{t("import.problems.duplicate", { title: f.duplicateOf.title })}</Tag>
            <button type="button" onClick={() => onOpen(f.duplicateOf!.meeting)} className="text-[12.5px] font-semibold text-accent hover:underline">
              {t("common.open")}
            </button>
          </span>
        )}
      </div>
      {blocked && !f.problems.includes("duplicate") && <span className="text-small self-center text-muted">{t("import.wontImport")}</span>}
      <Button size="sm" className="h-[30px] self-center" aria-label={t("import.removeFile", { name: f.name })} onClick={onRemove}>
        {t("common.remove")}
      </Button>
    </li>
  );
}

/** One Zoom recording: its participants' tracks, imported together as one meeting. */
function GroupRow({ unit, locale, onRemove, onSeparate, onOpen }: { unit: Unit; locale: Locale; onRemove: (id: string) => void; onSeparate: () => void; onOpen: (meeting: string) => void }) {
  const { t } = useTranslation();
  const first = unit.files[0]!;
  const blocked = unit.files.some((f) => !isImportable(f));
  const dup = unit.files.find((f) => f.problems.includes("duplicate") && f.duplicateOf)?.duplicateOf ?? null;
  const total = unit.files.reduce((n, f) => n + (f.sizeBytes ?? 0), 0);
  const meta = [first.title, first.startedAt != null ? formatDate(first.startedAt, locale) : null, first.durationMs != null ? formatClock(first.durationMs) : null, formatBytes(total, locale)]
    .filter(Boolean)
    .join(t("common.metaSep"));
  return (
    <li data-group={unit.id} data-blocked={blocked ? "true" : undefined} className={cn("flex flex-col gap-2 border-t border-line px-3.5 py-3 first:border-t-0", blocked && "bg-surface2")}>
      <div className="flex items-start gap-3">
        <Icon name={SOURCE_ICON.zoom} size={22} className="mt-0.5 flex-none text-muted" />
        <div className="min-w-0 flex-1">
          <b className={cn("block truncate text-[13.5px] font-semibold", blocked && "text-muted")}>{t("import.group.title", { count: unit.files.length })}</b>
          <span className="text-small block text-muted">{meta}</span>
          <span className="text-small block text-muted">{t("import.group.body", { app: APP_NAME })}</span>
          {dup && (
            <span className="text-small flex flex-wrap items-center gap-1.5">
              <Tag tone="warn">{t("import.problems.duplicate", { title: dup.title })}</Tag>
              <button type="button" onClick={() => onOpen(dup.meeting)} className="font-semibold text-accent hover:underline">
                {t("common.open")}
              </button>
            </span>
          )}
          {unit.files.some((f) => f.problems.includes("tooManyTracks")) && <Tag tone="bad">{word(t, KEYS.tooManyTracks)}</Tag>}
        </div>
        <Button variant="ghost" size="sm" onClick={onSeparate}>
          {word(t, KEYS.separately)}
        </Button>
      </div>
      <ul aria-label={t("import.group.title", { count: unit.files.length })} className="m-0 flex list-none flex-col gap-0.5 p-0 pl-9">
        {unit.files.map((f, i) => (
          <li key={f.id} className="flex items-center gap-2 text-[13px]">
            <Icon name="person" size={16} className="flex-none text-muted" />
            <span className="min-w-0 truncate font-medium">{f.participant ?? t("import.group.unnamed", { number: i + 1 })}</span>
            <span className="text-small min-w-0 flex-1 truncate text-faint">{f.name}</span>
            {f.problems.includes("unsupported") && <Tag tone="bad">{t("import.problems.unsupported")}</Tag>}
            {f.problems.includes("empty") && <Tag tone="bad">{t("import.problems.empty")}</Tag>}
            {f.problems.includes("veryLong") && <Tag tone="warn">{t("import.problems.veryLong")}</Tag>}
            <Button variant="ghost" size="sm" icon="close" aria-label={t("import.removeFile", { name: f.participant ?? f.name })} title={t("common.remove")} onClick={() => onRemove(f.id)} />
          </li>
        ))}
      </ul>
    </li>
  );
}

/** A failed import's message: the words for a code the core returns, else the message as it is. */
function errorWords(error: string | null, t: TFunction): string {
  if (error && (IMPORT_ERROR_CODES as readonly string[]).includes(error)) return (t as unknown as (key: string) => string)(`import.errors.${error}`);
  return error ?? "";
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
            : t("import.queue.failed", { message: errorWords(item.error, t) });
  return (
    <li data-state={item.state} className="flex flex-wrap items-center gap-3 rounded-panel border border-line px-3.5 py-3">
      <Icon
        name={item.state === "done" ? "check_circle" : item.state === "failed" ? "error" : item.state === "cancelled" ? "block" : "progress_activity"}
        size={20}
        className={cn("flex-none", item.state === "done" ? "text-accent" : "text-muted", item.state === "decoding" && "animate-spin motion-reduce:animate-none")}
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
