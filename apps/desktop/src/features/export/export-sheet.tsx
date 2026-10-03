// SPDX-License-Identifier: Apache-2.0
// Export sheet (§N): one or many meetings to Markdown, Word, text or
// subtitles, or copy / Obsidian for a single one. Files are written by the
// core after its own save dialog; the webview never sees a path.
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Dialog, DialogClose, Icon, cn, useToast, usePlatform } from "@ghi/ui";
import type { ExportFormat } from "../../bindings";
import { ipc } from "../../ipc";
import { FollowupEmailDialog } from "../email/followup-email-dialog";
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
      } else {
        const r = await ipc.commands.exportMeetings(meetings, format, content, t("export.title"));
        if (r.status === "error") return fail(r.error);
        if (r.data == null) return;
        saved(t("export.savedMany", { count: r.data }));
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

  return (
    <>
      <Dialog
        open={open}
        onOpenChange={onOpenChange}
        title={t("export.title")}
        description={meetings.length > 1 ? t("library.selected", { count: meetings.length }) : undefined}
        width={860}
        footer={
          <>
            <span className="text-small mr-auto flex items-center gap-1.5 self-center text-muted">
              <Icon name="lock" size={16} />
              {t("privacy.local")}
            </span>
            <DialogClose asChild>
              <Button>{t("common.cancel")}</Button>
            </DialogClose>
            <Button
              variant="primary"
              icon="ios_share"
              disabled={busy || meetings.length === 0 || !canExport(format, notes, transcript)}
              onClick={() => void save()}
            >
              {t("export.save")}
            </Button>
          </>
        }
      >
        <div className="grid min-h-0 grid-cols-[290px_minmax(0,1fr)] gap-5">
          <div className="flex min-w-0 flex-col">
            <span className="text-small mb-1.5 font-semibold text-muted">{t("export.format")}</span>
            <div
              role="radiogroup"
              aria-label={t("export.format")}
              className="flex flex-col gap-2"
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
                    "flex h-11 items-center gap-2.5 rounded-ctl border px-3 text-left text-[14px]",
                    format === f ? "border-accent bg-accent-soft font-semibold text-accent" : "border-ctl bg-surface hover:border-accent",
                  )}
                >
                  <Icon name={FORMAT_ICON[f]} size={18} />
                  {t(FORMAT_KEY[f])}
                </button>
              ))}
            </div>

            <fieldset className="m-0 mt-3 flex flex-col gap-0.5 border-0 p-0">
              <legend className="text-small mb-1 p-0 font-semibold text-muted">{t("export.include")}</legend>
              <Toggle label={t("export.parts.notes")} checked={!subtitles && notes} disabled={subtitles} onChange={setNotes} />
              <Toggle label={t("export.parts.transcript")} checked={subtitles || transcript} disabled={subtitles} onChange={setTranscript} />
              {subtitles && <p className="text-small m-0 mt-1 text-muted">{t("export.subtitlesNote")}</p>}
            </fieldset>

            <div className="mt-3 flex items-center justify-between gap-2">
              <span className="text-[14px] font-medium">{t("common.language")}</span>
              <div role="group" aria-label={t("common.language")} className="flex gap-0.5 rounded-ctl border border-ctl p-0.5">
                {LANGS.map((l) => (
                  <button
                    key={l}
                    type="button"
                    aria-pressed={language === l}
                    onClick={() => setLanguage(l)}
                    className={cn("h-6 rounded-seg px-2.5 text-[12px] font-semibold", language === l ? "bg-accent-soft text-accent" : "text-muted hover:text-ink")}
                  >
                    {l.toUpperCase()}
                  </button>
                ))}
              </div>
            </div>
            {single && (
              <div className="mt-3 flex flex-wrap items-center gap-2 border-t border-line pt-3">
                <Button icon="content_copy" disabled={busy || subtitles || !canExport(format, notes, transcript)} onClick={() => void copy(true)}>
                  {t("export.formats.markdown")}
                </Button>
                <Button icon="content_copy" disabled={busy || subtitles || !canExport(format, notes, transcript)} onClick={() => void copy(false)}>
                  {t("export.copyText")}
                </Button>
                <Button icon="folder" disabled={busy || subtitles || !canExport(format, notes, transcript)} onClick={() => void obsidian(false)}>
                  {t("export.toObsidian")}
                </Button>
                <Button
                  icon="inbox"
                  onClick={() => {
                    setEmailing(true);
                    onOpenChange(false);
                  }}
                >
                  {t("detail.more.draftEmail")}
                </Button>
                <button
                  type="button"
                  disabled={busy || subtitles}
                  onClick={() => void obsidian(true)}
                  className="text-small h-6 rounded-seg px-1 font-semibold text-accent hover:underline disabled:opacity-50"
                >
                  {t("export.changeFolder")}
                </button>
              </div>
            )}
          </div>
          <div className="flex min-h-0 min-w-0 flex-col gap-1.5">
            <div className="flex items-baseline justify-between gap-2">
              <span className="text-[13px] font-semibold text-muted">{t("export.preview")}</span>
              <span className="text-mono text-faint">{t(FORMAT_KEY[format])}</span>
            </div>
            {/* Text node only: the notes inside are never parsed (RT-6). */}
            <pre
              data-testid="export-preview"
              className="text-mono m-0 h-[380px] overflow-auto rounded-row border border-line bg-surface2 p-3.5 whitespace-pre-wrap break-words text-ink"
            >
              {preview.isError ? t("system.commandFailed", { message: preview.error.message }) : (preview.data ?? (subtitles ? t("export.subtitlesNote") : ""))}
            </pre>
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
      className="flex h-8 items-center gap-2 rounded-seg px-2 text-left text-[13px] hover:bg-surface2 disabled:opacity-60 disabled:hover:bg-transparent"
    >
      <Icon name={checked ? "check_box" : "check_box_outline_blank"} size={18} className={checked ? "text-accent" : "text-muted"} />
      {label}
    </button>
  );
}
