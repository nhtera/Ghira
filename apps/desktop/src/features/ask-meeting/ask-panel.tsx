// SPDX-License-Identifier: Apache-2.0
// "Ask this meeting" (D9-lite): a side panel on the detail screen. Answers come
// from the on-device model by default, or from a cloud provider through the
// same preview-and-send sheet. Each answer links to the moments it came from.
// Only the last few questions are kept, in component state.
import { Button, Icon, Segmented, useToast, usePlatform } from "@ghi/ui";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AskAnswer, MeetingDetail } from "../../bindings";
import { ipc } from "../../ipc";
import { AskError } from "../ask/ask-error";
import { CitationGroup } from "../citation/citation-link";
import { useCloudOffered } from "../cloud-sheet/cloud-offered";
import { CloudSheet } from "../cloud-sheet/cloud-sheet";
import { providerName } from "../cloud-sheet/provider-names";
import { invalidateMeeting, useMeetingNotes } from "../../state/meeting-queries";
import { STARTER_MAX, openQuestionStarters } from "./starters";

const KEEP = 5;
const STATIC_STARTERS = ["decided", "actions", "themes"] as const;

type Entry = {
  id: number;
  question: string;
  startedAt: number;
  state: "thinking" | "done" | "error";
  answer?: AskAnswer;
  error?: string;
  /** The cloud request failed and this answer came from the device instead. */
  cloudFailed?: string;
};

type Engine = "local" | "cloud";

function Elapsed({ since }: { since: number }) {
  const { t } = useTranslation();
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 500);
    return () => window.clearInterval(timer);
  }, []);
  return (
    <>
      {t("ask.meeting.thinking", {
        seconds: Math.max(0, Math.floor((now - since) / 1000)),
      })}
    </>
  );
}

function AnswerCard({
  entry,
  detail,
  meeting,
}: {
  entry: Entry;
  detail: MeetingDetail;
  meeting: string;
}) {
  const { t } = useTranslation();
  const a = entry.answer;
  const { show } = useToast();
  const client = useQueryClient();
  const [saved, setSaved] = useState(false);
  // A second click while the first is on its way would ask again.
  const [saving, setSaving] = useState(false);
  const save = async () => {
    if (!a?.id || saving) return;
    setSaving(true);
    const r = await ipc.commands.saveAnswer(meeting, a.id).finally(() => setSaving(false));
    if (r.status === "ok") {
      setSaved(true);
      void invalidateMeeting(client, meeting);
      return show({ tone: "success", title: t("ask.meeting.saved") });
    }
    const why = r.error === "answerExpired" ? t("ask.meeting.saveExpired") : r.error === "answerLimit" ? t("ask.meeting.saveLimit") : t("system.commandFailed", { message: r.error });
    show({ tone: "warning", title: why });
  };
  return (
    <li data-testid="ask-entry" className="flex flex-col gap-2">
      <p className="text-body m-0 self-end rounded-panel bg-sunk px-3 py-1.5 font-semibold">
        {entry.question}
      </p>
      {entry.state === "thinking" && (
        <p
          role="status"
          className="text-small m-0 flex items-center gap-2 text-muted"
        >
          <Icon
            name="progress_activity"
            size={16}
            className="animate-spin motion-reduce:animate-none"
          />
          <Elapsed since={entry.startedAt} />
        </p>
      )}
      {entry.state === "error" && <AskError error={entry.error ?? ""} />}
      {entry.state === "done" && a && !a.answered && (
        <div
          data-testid="ask-not-discussed"
          className="flex flex-col gap-1.5 rounded-panel border border-dashed border-line2 bg-surface p-3"
        >
          <b className="text-body flex items-center gap-1.5 font-semibold">
            <Icon name="search_off" size={18} className="text-muted" />
            {t("ask.meeting.notDiscussed")}
          </b>
          {a.searched.length > 0 && (
            <p className="text-small m-0 text-muted">
              {t("ask.meeting.searched")} {a.searched.join(", ")}
            </p>
          )}
        </div>
      )}
      {entry.state === "done" && a?.answered && (
        <div className="flex flex-col gap-2 rounded-panel border border-line2 bg-surface p-3">
          {/* Text node only: the answer is model output (RT-6). */}
          <p className="text-body m-0 whitespace-pre-wrap">{a.text}</p>
          {a.citations.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              <CitationGroup
                citations={a.citations}
                speakers={detail.speakers}
                audioAvailable={detail.audioAvailable}
                meeting={meeting}
              />
            </div>
          )}
          {a.id && (
            <Button size="sm" variant="ghost" icon={saved ? "check" : "bookmark"} disabled={saved || saving} onClick={() => void save()} className="self-start">
              {saved ? t("ask.meeting.saved") : t("ask.meeting.save")}
            </Button>
          )}
        </div>
      )}
      {entry.state === "done" && a && (
        <p className="text-small m-0 flex items-center gap-1.5 text-muted">
          <Icon
            name={a.engine === "local" ? "lock" : "cloud"}
            size={14}
            className={a.engine === "local" ? "text-accent" : "text-warn"}
          />
          {a.engine === "local"
            ? t("ask.meeting.answeredLocal")
            : t("ask.meeting.byProvider", { provider: providerName(a.engine) })}
          {entry.cloudFailed &&
            ` · ${t("ask.meeting.cloudFailed", { reason: entry.cloudFailed })}`}
        </p>
      )}
      {entry.state === "done" && a?.answered && (
        <p data-testid="ask-commitment" className="text-small m-0 text-muted">
          {t("notes.proposalFootnote")}
        </p>
      )}
    </li>
  );
}

export function AskPanel({
  meeting,
  detail,
  onClose,
}: {
  meeting: string;
  detail: MeetingDetail;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const offered = useCloudOffered();
  const [question, setQuestion] = useState("");
  const [engine, setEngine] = useState<Engine>("local");
  const [entries, setEntries] = useState<Entry[]>([]);
  const [cloudQuestion, setCloudQuestion] = useState<string | null>(null);
  const nextId = useRef(0);
  const logEnd = useRef<HTMLDivElement>(null);
  const thinking = entries.some((e) => e.state === "thinking");
  // The meeting's own open questions first, the static ones to make up three.
  const notes = useMeetingNotes(meeting);
  const starters = useMemo(() => {
    const own = openQuestionStarters(notes.data?.blocks ?? []);
    const fixed = STATIC_STARTERS.slice(0, Math.max(0, STARTER_MAX - own.length)).map((k) => t(`ask.meeting.suggestions.${k}`));
    return [...own, ...fixed];
  }, [notes.data, t]);

  useEffect(() => {
    logEnd.current?.scrollIntoView?.({ block: "end" });
  }, [entries]);

  const patch = (id: number, p: Partial<Entry>) =>
    setEntries((es) => es.map((e) => (e.id === id ? { ...e, ...p } : e)));
  const add = (q: string): number => {
    const id = ++nextId.current;
    setEntries((es) =>
      [
        ...es,
        { id, question: q, startedAt: Date.now(), state: "thinking" as const },
      ].slice(-KEEP),
    );
    return id;
  };

  const askLocal = async (
    q: string,
    existing?: number,
    cloudFailed?: string,
  ) => {
    const id = existing ?? add(q);
    const r = await ipc.commands.askMeeting(meeting, q, "meeting");
    if (r.status === "error") patch(id, { state: "error", error: r.error });
    else patch(id, { state: "done", answer: r.data, cloudFailed });
  };

  const submit = (text = question) => {
    const q = text.trim();
    if (!q || thinking) return;
    setQuestion("");
    if (offered && engine === "cloud") setCloudQuestion(q);
    else void askLocal(q);
  };

  return (
    <aside
      aria-label={t("ask.meeting.title")}
      data-testid="ask-panel"
      className="flex h-full w-[340px] shrink-0 flex-col border-l border-line bg-surface2"
    >
      <div className="flex items-center gap-2 px-4 pt-4 pb-2">
        <Icon name="forum" size={18} className="text-muted" />
        <h2 className="text-heading m-0 flex-1">{t("ask.meeting.title")}</h2>
        <Button
          size="sm"
          variant="ghost"
          icon="close"
          aria-label={t("ask.meeting.close")}
          onClick={onClose}
        />
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-4 py-2">
        {entries.length === 0 ? (
          <div className="flex flex-col gap-2">
            <p className="text-small m-0 text-muted">{t("ask.meeting.hint")}</p>
            <ul aria-label={t("ask.meeting.suggestions.label")} className="m-0 flex list-none flex-col items-start gap-1.5 p-0">
              {starters.map((q) => (
                <li key={q}>
                  <Button size="sm" disabled={thinking} onClick={() => submit(q)} className="h-auto min-h-8 py-1 text-left whitespace-normal">
                    {q}
                  </Button>
                </li>
              ))}
            </ul>
          </div>
        ) : (
          <ol
            aria-live="polite"
            className="m-0 flex list-none flex-col gap-4 p-0"
          >
            {entries.map((e) => (
              <AnswerCard key={e.id} entry={e} detail={detail} meeting={meeting} />
            ))}
          </ol>
        )}
        <div ref={logEnd} />
      </div>
      <div className="flex flex-col gap-2 border-t border-line px-4 py-3">
        {offered && (
        <Segmented<Engine>
          label={t("ask.meeting.engine")}
          value={engine}
          onChange={setEngine}
          options={[
            {
              value: "local",
              label: t("ask.meeting.onDevice", { context: platform }),
              icon: "lock",
            },
            { value: "cloud", label: t("ask.meeting.cloud"), icon: "cloud" },
          ]}
        />
        )}
        <div className="flex items-center gap-2">
          <input
            // Opened on purpose (toolbar or menu): ready to type.
            autoFocus
            value={question}
            aria-label={t("ask.meeting.label")}
            placeholder={t("ask.meeting.placeholder")}
            onChange={(e) => setQuestion(e.target.value)}
            // Enter confirms an IME candidate too; only a plain Enter sends.
            onKeyDown={(e) => {
              if (
                e.key === "Enter" &&
                !e.nativeEvent.isComposing &&
                e.keyCode !== 229
              ) {
                e.preventDefault();
                submit();
              }
            }}
            className="text-body h-8 min-w-0 flex-1 rounded-ctl border border-ctl bg-surface px-2.5 text-ink focus-visible:outline-2 focus-visible:outline-accent"
          />
          <Button
            variant="primary"
            size="sm"
            icon="arrow_upward"
            aria-label={t("ask.meeting.send")}
            disabled={!question.trim() || thinking}
            onClick={() => submit()}
          />
        </div>
      </div>
      <CloudSheet
        open={cloudQuestion != null}
        onOpenChange={(o) => !o && setCloudQuestion(null)}
        meeting={meeting}
        locked={detail.cloudLocked || detail.sensitive}
        task={{ kind: "ask", question: cloudQuestion ?? "" }}
        onAnswer={(answer) => {
          const q = cloudQuestion ?? "";
          const id = add(q);
          patch(id, { state: "done", answer });
        }}
        onAskFailed={(reason) =>
          void askLocal(cloudQuestion ?? "", undefined, reason)
        }
      />
    </aside>
  );
}
