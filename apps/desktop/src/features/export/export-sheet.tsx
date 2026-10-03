// SPDX-License-Identifier: Apache-2.0
// Export sheet (§N): one or many meetings to Markdown, Word, text or
// subtitles, or copy / Obsidian for a single one. Files are written by the
// core after its own save dialog; the webview never sees a path.
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Dialog, DialogClose, Icon, cn, useToast, usePlatform } from "@ghi/ui";
import type { ExportFormat } from "../../bindings";
import { ipc } from "../../ipc";
import { FollowupEmailDialog } from "../email/followup-email-dialog";
import { refreshExportDestination, useExportDestination } from "./destination";
import { FORMATS, canExport, exportContent, isSubtitles } from "./export-options";

const LANGS = ["en", "vi"] as const;
const FORMAT_ICON = { markdown: "description", docx: "description", text: "description", srt: "subtitles", vtt: "subtitles" } as const;

const FORMAT_KEY = {
  markdown: "export.fmt.markdown",
  docx: "export.formats.docx",
  text: "export.formats.txt",
  srt: "export.fmt.srt",
  vtt: "export.fmt.vtt",
} as const;

export function ExportSheet({ open, onOpenChange, meetings }: { open: boolean; onOpenChange: (open: boolean) => void; meetings: string[] }) {
  const { t, i18n } = useTranslation();
  const platform = usePlatform();
  const { show } = useToast();
  const [format, setFormat] = useState<ExportFormat>("markdown");
  const [notes, setNotes] = useState(true);
  const [transcript, setTranscript] = useState(false);
  const [busy, setBusy] = useState(false);
  const [emailing, setEmailing] = useState(false);
  // The headings' language: the app's, until changed here.
  const [language, setLanguage] = useState(i18n.language.startsWith("vi") ? "vi" : "en");
  const destination = useExportDestination(open, t("export.title"));
  const client = useQueryClient();
  const single = meetings.length === 1 ? meetings[0]! : null;
  const subtitles = isSubtitles(format);
  const content = exportContent(format, notes, transcript, language);

  // The text the file would hold, made on this device (subtitles have no text form).
  const preview = useQuery({
    queryKey: ["export-preview", single, format === "markdown", content.notes, content.transcript, content.vietnamese],
    enabled: open && !!single && !subtitles && canExport(format, notes, transcript),
    queryFn: async () => {
      const r = await ipc.commands.meetingAsText(single!, format === "markdown", content);
      if (r.status === "error") throw new Error(r.error);
      return r.data;
    },
  });

  const fail = (message: string) => show({ tone: "warning", title: t("system.commandFailed", { message }) });
  const saved = (title: string) =>
    show({
      tone: "success",
      title,
      action: {
        label: t("export.reveal", { context: platform }),
        altText: t("export.reveal", { context: platform }),
        onAction: () => void ipc.commands.revealLastExport(),
      },
    });

  const changeDestination = async () => {
    const error = await destination.change();
    if (error) fail(error);
  };

  const run = async (job: () => Promise<void>) => {
    setBusy(true);
    try {
      await job();
    } finally {
      setBusy(false);
    }
  };

  const save = () =>
    run(async () => {
      if (single) {
        const r = await ipc.commands.exportMeeting(single, format, content);
        if (r.status === "error") return fail(r.error);
        if (r.data == null) return; // the save dialog was cancelled
        saved(t("export.saved", { name: r.data }));
        void refreshExportDestination(client);
      } else {
        const r = await ipc.commands.exportMeetings(meetings, format, content, t("export.title"));
        if (r.status === "error") return fail(r.error);
        if (r.data == null) return;
        saved(t("export.savedMany", { count: r.data }));
        void refreshExportDestination(client);
      }
      onOpenChange(false);
    });

  const copy = (markdown: boolean) =>
    run(async () => {
      if (!single) return;
      const r = await ipc.commands.meetingAsText(single, markdown, content);
      if (r.status === "error") return fail(r.error);
      try {
        await navigator.clipboard.writeText(r.data);
        show({ tone: "success", title: t("export.copied") });
      } catch (e) {
        fail(String(e));
      }
    });

  const obsidian = (chooseFolder: boolean) =>
    run(async () => {
      if (!single) return;
      const r = await ipc.commands.exportObsidian(single, content, chooseFolder, t("export.formats.obsidian"));
      if (r.status === "error") return fail(r.error);
      if (r.data == null) return;
      saved(t("export.saved", { name: r.data }));
      onOpenChange(false);
    });

  const copyable = !!single && !subtitles && canExport(format, notes, transcript);
  const goTo = destination.folder;

  return (
    <>
      <Dialog
        open={open}
        onOpenChange={onOpenChange}
        title={<span className="sr-only">{t("export.title")}</span>}
        width={860}
      >
        {/* The design's sheet: options at the left, the preview and the buttons at the right; edge to edge inside the dialog's padding. */}
        <div className="-mx-5 -mt-8 -mb-5 grid h-[600px] max-h-[calc(85vh-2px)] min-h-0 grid-cols-[290px_minmax(0,1fr)]">
          <div className="flex min-w-0 flex-col gap-4 overflow-y-auto border-r border-line p-5">
            <b aria-hidden="true" className="text-[17px]">
              {t("export.title")}
              {meetings.length > 1 && <span className="ml-2 text-[12.5px] font-normal text-muted">{t("library.selected", { count: meetings.length })}</span>}
            </b>
            <div className="flex flex-col gap-[5px]">
              <span className="text-[12px] font-semibold text-muted">{t("export.format")}</span>
              <div
                role="radiogroup"
                aria-label={t("export.format")}
                className="flex flex-col gap-[5px]"
                // Arrow keys move the choice (one Tab stop for the group).
                onKeyDown={(e) => {
                  const step = e.key === "ArrowDown" || e.key === "ArrowRight" ? 1 : e.key === "ArrowUp" || e.key === "ArrowLeft" ? -1 : 0;
                  if (!step) return;
                  e.preventDefault();
                  const next = FORMATS[(FORMATS.indexOf(format) + step + FORMATS.length) % FORMATS.length]!;
                  setFormat(next);
                  e.currentTarget.querySelector<HTMLElement>(`[data-format="${next}"]`)?.focus();
                }}
              >
                {FORMATS.map((f) => (
                  <button
                    key={f}
                    type="button"
                    role="radio"
                    data-format={f}
                    aria-checked={format === f}
                    tabIndex={format === f ? 0 : -1}
                    onClick={() => setFormat(f)}
                    className={cn(
                      "flex h-[38px] items-center gap-2.5 rounded-ctl border-[1.5px] px-2.5 text-left text-[13px]",
                      format === f ? "border-accent bg-accent-soft font-semibold text-accent" : "border-ctl bg-surface hover:border-accent",
                    )}
                  >
                    <Icon name={FORMAT_ICON[f]} size={18} />
                    {t(FORMAT_KEY[f])}
                  </button>
                ))}
              </div>
            </div>

            <fieldset className="m-0 flex flex-col border-0 p-0">
              <legend className="mb-1 p-0 text-[12px] font-semibold text-muted">{t("export.include")}</legend>
              <Toggle label={t("export.parts.notes")} checked={!subtitles && notes} disabled={subtitles} onChange={setNotes} />
              <Toggle label={t("export.parts.transcript")} checked={subtitles || transcript} disabled={subtitles} onChange={setTranscript} />
              {subtitles && <p className="text-small m-0 mt-1 text-muted">{t("export.subtitlesNote")}</p>}
            </fieldset>

            <div className="flex items-center gap-2.5">
              <span className="flex-1 text-[13px]">{t("common.language")}</span>
              <div role="group" aria-label={t("common.language")} className="flex gap-0.5 rounded-ctl border border-ctl p-0.5">
                {LANGS.map((l) => (
                  <button
                    key={l}
                    type="button"
                    aria-pressed={language === l}
                    onClick={() => setLanguage(l)}
                    className={cn("h-6 rounded-seg px-[9px] text-[12px] font-semibold", language === l ? "bg-accent-soft text-accent" : "text-muted hover:text-ink")}
                  >
                    {l.toUpperCase()}
                  </button>
                ))}
              </div>
            </div>

            {single && (
              <div className="flex flex-col items-start gap-1.5 border-t border-line pt-3">
                <Button size="sm" icon="content_copy" disabled={busy || !copyable} onClick={() => void copy(true)}>
                  {t("export.formats.markdown")}
                </Button>
                <Button size="sm" icon="content_copy" disabled={busy || !copyable} onClick={() => void copy(false)}>
                  {t("export.copyText")}
                </Button>
                <Button size="sm" icon="folder" disabled={busy || !copyable} onClick={() => void obsidian(false)}>
                  {t("export.toObsidian")}
                </Button>
                <button
                  type="button"
                  disabled={busy || subtitles}
                  onClick={() => void obsidian(true)}
                  className="text-small h-6 rounded-seg px-1 font-semibold text-accent hover:underline disabled:opacity-50"
                >
                  {t("export.changeFolder")}
                </button>
                <Button
                  size="sm"
                  icon="inbox"
                  onClick={() => {
                    setEmailing(true);
                    onOpenChange(false);
                  }}
                >
                  {t("detail.more.draftEmail")}
                </Button>
              </div>
            )}
          </div>

          <div className="flex min-h-0 min-w-0 flex-col">
            <div className="flex items-center gap-2 px-[18px] pt-4 pb-2 text-[12px] font-semibold text-muted">
              {t("export.preview")}
              <span className="flex-1" />
              <span className="text-mono font-normal text-faint">{t(FORMAT_KEY[format])}</span>
            </div>
            {/* Text node only: the notes inside are never parsed (RT-6). */}
            <pre
              data-testid="export-preview"
              className="text-mono m-0 mx-[18px] min-h-0 flex-1 overflow-auto rounded-row border border-line bg-surface2 p-3.5 text-[12px] leading-[1.65] whitespace-pre-wrap break-words text-ink"
            >
              {preview.isError ? t("system.commandFailed", { message: preview.error.message }) : (preview.data ?? (subtitles ? t("export.subtitlesNote") : ""))}
            </pre>
            <div className="flex flex-wrap items-center gap-x-1.5 px-[18px] pt-3 text-[12.5px] text-muted" data-testid="export-destination">
              <Icon name="folder" size={16} />
              {goTo ? (
                <>
                  {t("export.saveTo")} <b className="font-semibold text-ink">{goTo}</b>
                </>
              ) : (
                t("export.askWhere")
              )}
              <span aria-hidden="true">·</span>
              <button type="button" disabled={busy} onClick={() => void changeDestination()} className="h-6 rounded-seg font-semibold text-accent hover:underline disabled:opacity-50">
                {t("export.changeDestination")}
              </button>
            </div>
            <div className="flex items-center gap-2 px-[18px] py-3.5">
              <span className="flex flex-1 items-center gap-[5px] text-[12px] text-muted">
                <Icon name="lock" size={15} className="text-accent" />
                {t("privacy.local")}
              </span>
              <DialogClose asChild>
                <Button size="lg">{t("common.cancel")}</Button>
              </DialogClose>
              <Button
                variant="primary"
                size="lg"
                icon="ios_share"
                disabled={busy || meetings.length === 0 || !canExport(format, notes, transcript)}
                onClick={() => void save()}
              >
                {t("export.save")}
              </Button>
            </div>
          </div>
        </div>
      </Dialog>
      {single && <FollowupEmailDialog open={emailing} onOpenChange={setEmailing} meeting={single} />}
    </>
  );
}

function Toggle({ label, checked, disabled, onChange }: { label: string; checked: boolean; disabled?: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className="flex h-[30px] items-center gap-2 rounded-seg text-left text-[13px] hover:bg-surface2 disabled:opacity-60 disabled:hover:bg-transparent"
    >
      <Icon name={checked ? "check_box" : "check_box_outline_blank"} size={18} className={checked ? "text-accent" : "text-muted"} />
      {label}
    </button>
  );
}
