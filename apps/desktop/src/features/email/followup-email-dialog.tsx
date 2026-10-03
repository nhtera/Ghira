// SPDX-License-Identifier: Apache-2.0
// Follow-up email draft (phase 11): the local model writes a subject and body
// from the meeting's notes; the user edits and copies it. Nothing is sent.
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Dialog, DialogClose, Icon, cn, useToast } from "@ghi/ui";
import type { EmailTone, NotesLanguage } from "../../bindings";
import { ipc } from "../../ipc";
import { AskError } from "../ask/ask-error";
import { emailText } from "./email-text";

/** A radio group of bordered buttons (the design's Tone and Language rows). */
function Choice<T extends string>({ label, value, onChange, options, className }: { label: string; value: T; onChange: (v: T) => void; options: { value: T; label: string }[]; className?: string }) {
  return (
    <div role="radiogroup" aria-label={label} className={cn("flex gap-1.5", className)}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          onClick={() => onChange(o.value)}
          className={cn(
            "h-9 flex-1 rounded-ctl border px-2.5 text-[13px] font-medium",
            value === o.value ? "border-accent bg-accent-soft font-semibold text-accent" : "border-ctl bg-surface hover:border-accent",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

type Phase = { kind: "form" } | { kind: "writing" } | { kind: "draft" };

export function FollowupEmailDialog({ open, onOpenChange, meeting }: { open: boolean; onOpenChange: (open: boolean) => void; meeting: string }) {
  const { t } = useTranslation();
  const { show } = useToast();
  const [language, setLanguage] = useState<NotesLanguage>("meeting");
  const [tone, setTone] = useState<EmailTone>("friendly");
  const [phase, setPhase] = useState<Phase>({ kind: "form" });
  const [subject, setSubject] = useState("");
  const [body, setBody] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [seconds, setSeconds] = useState(0);
  // Only the latest request may apply its answer (closing or rewriting outruns an older one).
  const request = useRef(0);

  useEffect(() => {
    if (phase.kind !== "writing") return;
    const id = setInterval(() => setSeconds((s) => s + 1), 1000);
    return () => clearInterval(id);
  }, [phase.kind]);

  const write = async () => {
    const mine = ++request.current;
    setError(null);
    setSeconds(0);
    setPhase({ kind: "writing" });
    const r = await ipc.commands.draftFollowupEmail(meeting, language, tone);
    if (mine !== request.current) return;
    if (r.status === "error") {
      setError(r.error);
      setPhase({ kind: subject || body ? "draft" : "form" });
      return;
    }
    setSubject(r.data.subject);
    setBody(r.data.body);
    setPhase({ kind: "draft" });
  };

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(emailText(subject, body));
      show({ tone: "success", title: t("email.copied") });
    } catch (e) {
      show({ tone: "warning", title: t("system.commandFailed", { message: String(e) }) });
    }
  };

  const change = (o: boolean) => {
    if (!o) request.current++; // drop a draft still being written
    if (!o && phase.kind === "writing") setPhase({ kind: subject || body ? "draft" : "form" });
    onOpenChange(o);
  };

  const writing = phase.kind === "writing";
  const hasDraft = subject !== "" || body !== "";
  return (
    <Dialog
      open={open}
      onOpenChange={change}
      title={t("email.title")}
      width={860}
      footer={
        <>
          <span className="text-small mr-auto flex items-center gap-1.5 self-center text-muted">
            <Icon name="lock" size={16} />
            {t("email.notSent")}
          </span>
          <DialogClose asChild>
            <Button>{t("common.cancel")}</Button>
          </DialogClose>
          {hasDraft && (
            <Button variant="primary" icon="content_copy" disabled={writing} onClick={() => void copy()}>
              {t("email.copy")}
            </Button>
          )}
        </>
      }
    >
      <div className="grid min-h-0 grid-cols-[290px_minmax(0,1fr)] gap-5">
        <div className="flex min-w-0 flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <span className="text-small font-semibold text-muted">{t("email.tone")}</span>
            <Choice<EmailTone>
              label={t("email.tone")}
              value={tone}
              onChange={setTone}
              options={[
                { value: "friendly", label: t("email.tones.friendly") },
                { value: "neutral", label: t("email.tones.neutral") },
                { value: "formal", label: t("email.tones.formal") },
              ]}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <span className="text-small font-semibold text-muted">{t("common.language")}</span>
            <Choice<NotesLanguage>
              label={t("common.language")}
              value={language}
              onChange={setLanguage}
              options={[
                { value: "meeting", label: t("meeting.lang.meeting") },
                { value: "en", label: t("import.options.languages.english") },
                { value: "vi", label: t("import.options.languages.vietnamese") },
              ]}
            />
          </div>
          <div className="flex items-center gap-3">
            <Button variant={hasDraft ? "secondary" : "primary"} icon={hasDraft ? "refresh" : "auto_awesome"} disabled={writing} onClick={() => void write()}>
              {hasDraft ? t("email.rewrite") : t("email.write")}
            </Button>
            {writing && (
              <span role="status" className="text-small text-muted">
                {t("email.writing", { seconds })}
              </span>
            )}
          </div>
          {error && <AskError error={error} status />}
        </div>

        <div className="flex min-h-[360px] min-w-0 flex-col">
          {hasDraft ? (
            <>
              <label className="flex items-baseline gap-3 border-b border-line pb-2">
                <span className="w-14 flex-none text-[13px] text-muted">{t("email.subject")}</span>
                <input
                  value={subject}
                  disabled={writing}
                  onChange={(e) => setSubject(e.target.value)}
                  className="min-w-0 flex-1 bg-transparent text-[14.5px] font-semibold text-ink outline-none focus-visible:underline"
                />
              </label>
              <label className="flex min-h-0 flex-1 flex-col pt-3">
                <span className="sr-only">{t("email.body")}</span>
                <textarea
                  value={body}
                  rows={14}
                  disabled={writing}
                  onChange={(e) => setBody(e.target.value)}
                  className="min-h-0 flex-1 resize-none bg-transparent font-serif text-[17px] leading-[1.7] text-ink outline-none"
                />
              </label>
            </>
          ) : (
            <p className="text-small m-0 flex flex-1 items-center justify-center text-muted">{writing ? t("email.writing", { seconds }) : null}</p>
          )}
        </div>
      </div>
    </Dialog>
  );
}
