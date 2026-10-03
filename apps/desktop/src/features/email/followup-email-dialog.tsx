// SPDX-License-Identifier: Apache-2.0
// Follow-up email draft (phase 11): the local model writes a subject and body
// from the meeting's notes; the user edits and copies it. Nothing is sent.
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar, Button, Dialog, DialogClose, Icon, cn, useToast, usePlatform } from "@ghi/ui";
import type { EmailTone } from "../../bindings";
import { ipc } from "../../ipc";
import { AskError } from "../ask/ask-error";
import { useQueryClient } from "@tanstack/react-query";
import type { MeetingDetail } from "../../bindings";
import { meetingKeys } from "../../state/meeting-queries";
import { useLlmName } from "../meeting/llm-name";
import { useNotesLanguage, type PickedLanguage } from "../meeting/notes-language";
import { openMailDraft, useMeetingContacts } from "./mail";
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
            "h-[30px] flex-1 rounded-[7px] border px-1.5 text-[12px] font-medium whitespace-nowrap",
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
const LANGS = ["en", "vi"] as const;

export function FollowupEmailDialog({ open, onOpenChange, meeting }: { open: boolean; onOpenChange: (open: boolean) => void; meeting: string }) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const { show } = useToast();
  // The detail screen already loaded it; from elsewhere the language falls back to the app's.
  const detail = useQueryClient().getQueryData<MeetingDetail>(meetingKeys.detail(meeting));
  const model = useLlmName();
  const contacts = useMeetingContacts(meeting, open);
  const [picked, setPicked] = useState<PickedLanguage | null>(null);
  const { shown, request: language } = useNotesLanguage(detail, picked);
  const [tone, setTone] = useState<EmailTone>("friendly");
  const [phase, setPhase] = useState<Phase>({ kind: "form" });
  const [subject, setSubject] = useState("");
  const [body, setBody] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [seconds, setSeconds] = useState(0);
  // Addresses left out of the "To" line; everyone with one is in until unticked.
  const [skipped, setSkipped] = useState<ReadonlySet<string>>(new Set());
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

  const people = (contacts.data ?? []).filter((c) => c.name.trim() !== "");
  // Someone without an address can't be a recipient: listed, but unticked and disabled.
  const recipients = people.filter((c) => c.email && !skipped.has(c.email));
  const addresses = recipients.flatMap((c) => (c.email ? [c.email] : []));

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(emailText(subject, body));
      show({ tone: "success", title: t("email.copied") });
    } catch (e) {
      show({ tone: "warning", title: t("system.commandFailed", { message: String(e) }) });
    }
  };

  const openInMail = async () => {
    const r = await openMailDraft(addresses, subject.trim(), body.trim());
    if ("error" in r) return show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
    // The mail app got a shortened body: the full email is one tap away (copied only then).
    if (r.truncated)
      show({
        tone: "info",
        title: t("email.truncated"),
        action: { label: t("email.copyFull"), altText: t("email.copyFull"), onAction: () => void copy() },
      });
  };

  const change = (o: boolean) => {
    if (!o) request.current++; // drop a draft still being written
    if (!o && phase.kind === "writing") setPhase({ kind: subject || body ? "draft" : "form" });
    onOpenChange(o);
  };

  const writing = phase.kind === "writing";
  const hasDraft = subject !== "" || body !== "";
  return (
    <Dialog open={open} onOpenChange={change} title={<span className="sr-only">{t("email.title")}</span>} width={860}>
      {/* The design's sheet: options at the left, the draft and the buttons at the right; edge to edge inside the dialog's padding. */}
      <div className="-mx-5 -mt-8 -mb-5 grid h-[600px] max-h-[calc(85vh-2px)] min-h-0 grid-cols-[290px_minmax(0,1fr)]">
        <div className="flex min-w-0 flex-col gap-4 overflow-y-auto border-r border-line p-5">
          <b aria-hidden="true" className="text-[17px]">
            {t("email.title")}
          </b>
          {people.length > 0 && (
            <div className="flex flex-col gap-1.5">
              <span className="text-[12px] font-semibold text-muted">{t("email.to")}</span>
              {people.map((c, i) => {
                const on = !!c.email && !skipped.has(c.email);
                return (
                  <button
                    key={c.email ?? `${i}-${c.name}`}
                    type="button"
                    role="checkbox"
                    aria-checked={on}
                    disabled={!c.email}
                    onClick={() =>
                      setSkipped((s) => {
                        const next = new Set(s);
                        if (!c.email) return next;
                        if (on) next.add(c.email);
                        else next.delete(c.email);
                        return next;
                      })
                    }
                    className={cn(
                      "flex h-10 items-center gap-2 rounded-[9px] border-[1.5px] py-0 pr-2.5 pl-1.5 text-left",
                      on ? "border-accent bg-accent-soft" : "border-ctl bg-surface enabled:hover:border-accent disabled:opacity-60",
                    )}
                  >
                    <Avatar kind="person" name={c.name} size="lg" colorSlot={detail?.speakers.find((x) => x.name === c.name)?.colorSlot ?? 0} />
                    <span className="grid min-w-0 flex-1">
                      <span className="truncate text-[13px] font-semibold">{c.name}</span>
                      {c.email && <span className="truncate text-[11.5px] text-muted">{c.email}</span>}
                    </span>
                    {on && <Icon name="check" size={17} className="text-muted" />}
                  </button>
                );
              })}
            </div>
          )}
          <div className="flex flex-col gap-1.5">
            <span className="text-[12px] font-semibold text-muted">{t("email.tone")}</span>
            <Choice<EmailTone>
              label={t("email.tone")}
              value={tone}
              onChange={setTone}
              className="gap-1"
              options={[
                { value: "friendly", label: t("email.tones.friendly") },
                { value: "neutral", label: t("email.tones.neutral") },
                { value: "formal", label: t("email.tones.formal") },
              ]}
            />
          </div>
          <div className="flex items-center gap-2.5">
            <span className="flex-1 text-[13px]">{t("common.language")}</span>
            <div role="group" aria-label={t("common.language")} className="flex gap-0.5 rounded-ctl border border-ctl p-0.5">
              {LANGS.map((l) => (
                <button
                  key={l}
                  type="button"
                  aria-pressed={shown === l}
                  // Any click sets the language explicitly.
                  onClick={() => setPicked(l)}
                  className={cn("h-6 rounded-seg px-2.5 text-[12px] font-semibold", shown === l ? "bg-accent-soft text-accent" : "text-muted hover:text-ink")}
                >
                  {l.toUpperCase()}
                </button>
              ))}
            </div>
          </div>
          <div className="flex items-center gap-3">
            <Button variant={hasDraft ? "secondary" : "primary"} size="lg" icon={hasDraft ? "refresh" : "auto_awesome"} disabled={writing} onClick={() => void write()}>
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

        <div className="flex min-h-0 min-w-0 flex-col px-5 pt-[18px]">
          {hasDraft ? (
            <>
              <div className="grid grid-cols-[64px_minmax(0,1fr)] items-baseline gap-x-2.5 gap-y-1.5 border-b border-line pb-3 text-[13px]">
                {recipients.length > 0 && (
                  <>
                    <span className="text-muted">{t("email.to")}</span>
                    <span className="truncate">{addresses.join(", ")}</span>
                  </>
                )}
                <label htmlFor="email-subject" className="text-muted">
                  {t("email.subject")}
                </label>
                <input
                  id="email-subject"
                  value={subject}
                  disabled={writing}
                  onChange={(e) => setSubject(e.target.value)}
                  className="min-w-0 bg-transparent font-semibold text-ink outline-none focus-visible:underline"
                />
              </div>
              <label className="flex min-h-0 flex-1 flex-col pt-3">
                <span className="sr-only">{t("email.body")}</span>
                <textarea
                  value={body}
                  rows={14}
                  disabled={writing}
                  onChange={(e) => setBody(e.target.value)}
                  className="min-h-0 flex-1 resize-none bg-transparent font-serif text-[15.5px] leading-[1.65] text-ink outline-none"
                />
              </label>
            </>
          ) : (
            <p className="text-small m-0 flex flex-1 items-center justify-center text-muted">{writing ? t("email.writing", { seconds }) : null}</p>
          )}
          <div className="flex items-center gap-2 border-t border-line py-3.5">
            <span className="flex flex-1 items-center gap-[5px] text-[12px] text-muted">
              <Icon name="lock" size={15} className="shrink-0 text-accent" />
              {model ? t("email.writtenLocally", { context: platform, model }) : t("email.notSent")}
            </span>
            <DialogClose asChild>
              <Button size="lg">{t("common.cancel")}</Button>
            </DialogClose>
            {hasDraft && (
              <>
                <Button size="lg" icon="content_copy" variant="secondary" disabled={writing} onClick={() => void copy()}>
                  {t("email.copy")}
                </Button>
                <Button variant="primary" size="lg" icon="inbox" disabled={writing} onClick={() => void openInMail()}>
                  {t("email.openMail", { context: platform })}
                </Button>
              </>
            )}
          </div>
        </div>
      </div>
    </Dialog>
  );
}
