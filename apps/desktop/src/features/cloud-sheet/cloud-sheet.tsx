// SPDX-License-Identifier: Apache-2.0
// "Improve with cloud…" / "Ask with cloud" send sheet (doc 02 §K). Shows the
// exact request that would leave the device (text only, never audio) and sends
// precisely that preview. Composed from the Dialog primitive: the shared
// CloudSendSheet shows a before/after excerpt, this one needs the real payload.
import {
  Button,
  Dialog,
  Icon,
  Segmented,
  Select,
  useToast,
  usePlatform,
} from "@ghi/ui";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import type {
  AskAnswer,
  CloudPreview,
  CloudTask,
  NotesLanguage,
} from "../../bindings";
import { ipc } from "../../ipc";
import { invalidateMeeting } from "../../state/meeting-queries";
import { userText } from "./user-text";
import { modelOption } from "./model-price";
import { providerName } from "./provider-names";
import {
  initialSelection,
  useCloudChoices,
  useCloudPreview,
  type Selection,
} from "./use-cloud-preview";

export type CloudSheetTask =
  | { kind: "notes"; template?: string | null; language?: NotesLanguage }
  | { kind: "ask"; question: string; language?: NotesLanguage };

export type CloudSheetProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meeting: string;
  /** Per-meeting "never send to cloud". */
  locked: boolean;
  task: CloudSheetTask;
  /** An Ask was answered (by the provider, or nothing matched). */
  onAnswer?: (answer: AskAnswer) => void;
  /** The provider failed an Ask: the caller answers on this device. */
  onAskFailed?: (reason: string) => void;
};

const toTask = (t: CloudSheetTask): CloudTask =>
  t.kind === "notes"
    ? {
        kind: "notes",
        template: t.template ?? null,
        language: t.language ?? "meeting",
      }
    : { kind: "ask", question: t.question, language: t.language ?? "meeting" };

/** A label and its value, like the rows of a receipt. */
function Fact({ label, labelFor, children }: { label: string; labelFor?: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[150px_minmax(0,1fr)] items-baseline gap-x-3 text-[14px]">
      {labelFor ? (
        <label htmlFor={labelFor} className="text-muted">
          {label}
        </label>
      ) : (
        <span className="text-muted">{label}</span>
      )}
      <span className="min-w-0">{children}</span>
    </div>
  );
}

/** Mounted only while open, so every opening starts from the saved defaults. */
export function CloudSheet(p: CloudSheetProps) {
  return p.open ? <CloudSheetBody {...p} /> : null;
}

function CloudSheetBody({
  open,
  onOpenChange,
  meeting,
  locked,
  task,
  onAnswer,
  onAskFailed,
}: CloudSheetProps) {
  const { t, i18n } = useTranslation();
  const platform = usePlatform();
  const navigate = useNavigate();
  const client = useQueryClient();
  const { show } = useToast();
  const redactId = useId();
  const modelId = useId();
  const choices = useCloudChoices(open);
  const [picked, setPicked] = useState<Selection | null>(null);
  const sel = picked ?? (choices.loaded ? initialSelection(choices) : null);
  const [sending, setSending] = useState(false);
  const [showExact, setShowExact] = useState(false);
  const [failure, setFailure] = useState<{
    reason: string;
    leftDevice: boolean;
  } | null>(null);
  const strict = choices.settings?.strictOffline ?? false;
  const blocked = locked || strict;
  const hasKey = !!sel && choices.stored.has(sel.provider);

  const apiTask = toTask(task);
  const { state, refresh } = useCloudPreview({
    meeting,
    task: apiTask,
    sel,
    enabled: open && !!sel && hasKey && !blocked && !failure,
  });

  // An Ask nothing matched is answered without a request: hand it over and close.
  const handled = useRef<unknown>(null);
  useEffect(() => {
    if (state.status === "answer" && handled.current !== state) {
      handled.current = state;
      onAnswer?.(state.answer);
      onOpenChange(false);
    }
  }, [state, onAnswer, onOpenChange]);

  const preview: CloudPreview | null =
    state.status === "ready" ? state.preview : null;
  const models = choices.models.filter((m) => m.provider === sel?.provider);
  const isAsk = task.kind === "ask";

  const pickProvider = (provider: string) =>
    setPicked((s) => {
      const list = choices.models.filter((m) => m.provider === provider);
      const keep = s && list.some((m) => m.model === s.model);
      return {
        redact: s?.redact ?? true,
        provider,
        model: keep ? s.model : (list[0]?.model ?? ""),
      };
    });

  const send = async () => {
    if (!preview || !sel) return;
    setSending(true);
    // Remember the choice as the next default, whatever happens to the send.
    void ipc.commands.updateSettings({
      cloudProvider: sel.provider,
      cloudModel: sel.model,
    });
    const r = await ipc.commands.cloudSend(preview.id);
    setSending(false);
    if (r.status === "error") {
      refresh(); // the preview may have expired
      return show({
        tone: "warning",
        title: t("system.commandFailed", { message: r.error }),
      });
    }
    const out = r.data;
    if (out.kind === "answer") {
      onAnswer?.(out);
      onOpenChange(false);
    } else if (out.kind === "notes") {
      show({ tone: "success", title: t("cloud.done") });
      void invalidateMeeting(client, meeting);
      onOpenChange(false);
    } else if (isAsk) {
      onAskFailed?.(out.reason);
      onOpenChange(false);
    } else {
      setFailure({ reason: out.reason, leftDevice: out.leftDevice });
      void invalidateMeeting(client, meeting); // the local rewrite has started
    }
  };

  const fmt = new Intl.NumberFormat(i18n.language);
  // The backend sends the user's text before and after redaction; without it, the payload's own text is "what is sent".
  const fullText = preview ? userText(preview.payload) : "";
  // "What is sent" shows the core's excerpt only when it really is part of the request body; otherwise the body's own text.
  const sentLines = preview?.excerptAfter?.split("\n").map((l) => l.trim()).filter(Boolean) ?? [];
  const excerptIsSent = sentLines.length > 0 && sentLines.every((l) => fullText.includes(l));
  const after = excerptIsSent && preview?.excerptAfter ? clip(preview.excerptAfter) : clip(fullText);
  const before = excerptIsSent && preview?.excerptBefore?.trim() ? clip(preview.excerptBefore) : null;
  const excerpt = after;
  const excerptWords = fullText.split(/\s+/).filter(Boolean).length;
  // The likely cost (a typical answer), and at most (the whole answer allowance).
  const usd = (v: number) =>
    v < 0.01
      ? `< ${new Intl.NumberFormat(i18n.language, { style: "currency", currency: "USD" }).format(0.01)}`
      : `≈ ${new Intl.NumberFormat(i18n.language, { style: "currency", currency: "USD", maximumFractionDigits: 2 }).format(v)}`;
  const cost =
    preview?.costEstUsd == null
      ? null
      : preview.costMaxUsd != null && preview.costMaxUsd >= preview.costEstUsd * 1.5
        ? t("cloud.costRange", { cost: usd(preview.costEstUsd), max: usd(preview.costMaxUsd).replace("≈ ", "") })
        : usd(preview.costEstUsd);
  const provider = sel ? providerName(sel.provider) : "";

  const footer = failure ? (
    <Button variant="primary" size="lg" onClick={() => onOpenChange(false)}>
      {t("cloud.close")}
    </Button>
  ) : (
    <>
      <Button
        variant="secondary"
        size="lg"
        icon="cloud_off"
        onClick={() => onOpenChange(false)}
        disabled={sending}
      >
        {t("cloud.keepLocal")}
      </Button>
      <Button
        variant="primary"
        size="lg"
        icon={sending ? "progress_activity" : "cloud_upload"}
        onClick={() => void send()}
        disabled={!preview || sending || blocked}
        aria-busy={sending}
      >
        {sending ? t("cloud.sendingButton") : t("cloud.send")}
      </Button>
    </>
  );

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={isAsk ? t("cloud.sheet.askTitle") : t("cloud.title")}
      description={t("cloud.subtitle")}
      dismissible={!sending}
      width={600}
      footer={footer}
    >
      <div className="flex flex-col gap-3.5">
        {blocked && (
          <p
            role="alert"
            className="text-body m-0 flex items-center gap-2 rounded-row bg-surface2 p-3 text-ink"
          >
            <Icon name="cloud_off" size={18} className="text-muted" />
            {locked ? t("cloud.sheet.locked") : t("cloud.sheet.strictOffline")}
          </p>
        )}

        {!blocked && !failure && sel && (
          <>
            <Fact label={t("cloud.provider")}>
              {/* A disabled fieldset disables every button inside it. */}
              <fieldset disabled={sending} className="m-0 min-w-0 border-0 p-0">
                <Segmented
                  label={t("cloud.provider")}
                  value={sel.provider}
                  onChange={pickProvider}
                  options={choices.providers.map((p) => ({
                    value: p,
                    label: providerName(p),
                  }))}
                />
              </fieldset>
            </Fact>
            <Fact label={t("cloud.sheet.model")} labelFor={modelId}>
              <Select
                id={modelId}
                className="w-[240px]"
                value={sel.model}
                onChange={(model) => setPicked({ ...sel, model })}
                disabled={sending || models.length === 0}
                options={models.map((m) => ({ value: m.model, label: modelOption(t, m, i18n.language) }))}
              />
            </Fact>
            {preview && <Fact label={t("cloud.sheet.destination")}>{preview.host}</Fact>}

            {!hasKey ? (
              <p
                role="status"
                className="text-body m-0 flex flex-wrap items-center gap-2 rounded-row bg-surface2 p-3"
              >
                <Icon name="info" size={18} className="text-muted" />
                <span>{t("cloud.sheet.noKey", { provider })}</span>
                <Button
                  size="sm"
                  onClick={() => {
                    onOpenChange(false);
                    void navigate({
                      to: "/settings/$section",
                      params: { section: "ai" },
                    });
                  }}
                >
                  {t("cloud.sheet.addKey")}
                </Button>
              </p>
            ) : (
              <>
                {preview && (
                  <>
                    <Fact label={t("cloud.whatLeaves", { context: platform })}>
                      {excerpt
                        ? t("cloud.textOnly", {
                            words: fmt.format(excerptWords),
                            tokens: fmt.format(preview.tokensEst),
                          })
                        : t("cloud.sheet.tokens", { tokens: fmt.format(preview.tokensEst) })}
                    </Fact>
                    <Fact label={t("cloud.audio")}>
                      <span className="inline-flex items-center gap-1.5 font-semibold text-accent">
                        <Icon name="lock" size={16} />
                        {t("cloud.audioNever")}
                      </span>
                    </Fact>
                    {cost && <Fact label={t("cloud.cost")}>{cost}</Fact>}
                  </>
                )}

                <label htmlFor={redactId} className="flex cursor-pointer items-center gap-2.5 text-[14px] font-medium">
                  <input
                    id={redactId}
                    type="checkbox"
                    role="switch"
                    checked={sel.redact}
                    disabled={sending}
                    onChange={(e) =>
                      setPicked({ ...sel, redact: e.target.checked })
                    }
                    className="peer sr-only"
                  />
                  <span
                    aria-hidden
                    // The Settings switch (parts.tsx): px, not rem, so the
                    // knob stays centred at any text size.
                    className="inline-flex h-[20px] w-[36px] shrink-0 items-center rounded-full bg-line2 p-[2px] transition-colors peer-checked:bg-accent peer-checked:[&>i]:translate-x-[16px] peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-accent peer-disabled:opacity-50"
                  >
                    <i className="block size-[16px] rounded-full bg-white shadow-[0_1px_2px_rgb(0_0_0/0.25)] transition-transform" />
                  </span>
                  {t("cloud.sheet.redact")}
                </label>

                {state.status === "loading" && (
                  <p
                    role="status"
                    className="text-small m-0 flex items-center gap-2 text-muted"
                  >
                    <Icon
                      name="progress_activity"
                      size={16}
                      className="animate-spin motion-reduce:animate-none"
                    />
                    {t("cloud.sheet.preparing")}
                  </p>
                )}
                {state.status === "error" && (
                  <p
                    role="alert"
                    className="text-small m-0 flex items-center gap-2 text-rec-ink"
                  >
                    <Icon name="error" size={16} />
                    {t("system.commandFailed", { message: state.message })}
                  </p>
                )}
                {preview && (
                  <>
                    {preview.warnings.length > 0 && (
                      <ul
                        aria-label={t("cloud.sheet.warnings")}
                        className="text-small m-0 flex list-none flex-col gap-1 p-0 text-warn"
                      >
                        {preview.warnings.map((w, i) => (
                          <li key={i} className="flex items-start gap-1.5">
                            <Icon
                              name="warning"
                              size={16}
                              className="shrink-0"
                            />
                            {w}
                          </li>
                        ))}
                      </ul>
                    )}
                    <div className="flex min-w-0 flex-col gap-1.5">
                      {excerpt && (
                        <div className={before ? "grid grid-cols-2 gap-2.5" : "grid gap-2.5"}>
                          {before && (
                            <div className="flex min-w-0 flex-col gap-1">
                              <span className="text-[11.5px] font-semibold text-faint">{t("cloud.before", { context: platform })}</span>
                              {/* Text node only: the transcript inside is never parsed as markup (RT-6). */}
                              <p
                                data-testid="cloud-excerpt-before"
                                className="text-mono m-0 max-h-32 overflow-hidden rounded-row border border-line bg-surface2 p-2.5 text-[11.5px] leading-[1.55] whitespace-pre-wrap break-words text-ink"
                              >
                                {before}
                              </p>
                            </div>
                          )}
                          <div className="flex min-w-0 flex-col gap-1">
                            <span className="text-[11.5px] font-semibold text-warn">{t("cloud.after")}</span>
                            <p
                              data-testid="cloud-excerpt"
                              className="text-mono m-0 max-h-32 overflow-hidden rounded-row bg-warn-soft p-2.5 text-[11.5px] leading-[1.55] whitespace-pre-wrap break-words text-ink"
                            >
                              {after}
                            </p>
                          </div>
                        </div>
                      )}
                      <span className="text-small text-faint">
                        {preview.redactions.length > 0
                          ? new Intl.ListFormat(i18n.language).format(
                              preview.redactions.map((r) =>
                                t("cloud.sheet.redacted", {
                                  count: r.count,
                                  kind: kindLabel(t, r.kind),
                                }),
                              ),
                            )
                          : sel.redact
                            ? t("cloud.sheet.noRedactions")
                            : t("cloud.sheet.redactionOff")}
                      </span>
                    </div>
                    {/* The whole request, byte for byte: what Send sends. Closed until asked for. */}
                    <details
                      open={showExact || !excerpt}
                      onToggle={(e) => setShowExact(e.currentTarget.open)}
                      className="min-w-0 text-[13px]"
                    >
                      <summary className="flex cursor-pointer items-center gap-1 text-muted select-none hover:text-ink">
                        <Icon name={showExact || !excerpt ? "expand_less" : "expand_more"} size={16} />
                        {t("cloud.sheet.showExact")}
                      </summary>
                      <div className="mt-1.5 flex min-w-0 flex-col gap-1">
                        <span id={`${redactId}-p`} className="text-label text-muted">
                          {t("cloud.sheet.payload")} · {t("cloud.sheet.host", { host: preview.host })}
                        </span>
                        <pre
                          tabIndex={0}
                          aria-labelledby={`${redactId}-p`}
                          data-testid="cloud-payload"
                          data-sha={preview.sha256}
                          className="text-mono m-0 max-h-52 overflow-auto rounded-ctl border border-line bg-sunk p-2.5 whitespace-pre-wrap break-words text-ink focus-visible:outline-2 focus-visible:outline-accent"
                        >
                          {preview.payload}
                        </pre>
                        <span className="text-small text-faint">
                          {t("cloud.sheet.sha")}{" "}
                          <span className="text-mono">{preview.sha256.slice(0, 12)}</span>
                        </span>
                      </div>
                    </details>
                    <p className="text-small m-0 text-muted">{t("cloud.sheet.retention", { provider: providerName(preview.provider) })}</p>
                  </>
                )}
              </>
            )}
          </>
        )}

        {sending && (
          <p
            role="status"
            className="text-small m-0 flex items-center gap-2 text-muted"
          >
            <Icon
              name="progress_activity"
              size={16}
              className="animate-spin motion-reduce:animate-none"
            />
            {t("cloud.sending")}
          </p>
        )}
        {failure && (
          <div
            role="alert"
            className="flex flex-col gap-1.5 rounded-row bg-surface2 p-3 text-[13px]"
          >
            <p className="m-0 flex items-center gap-2 font-semibold text-rec-ink">
              <Icon name="cloud_off" size={16} />
              {t("cloud.failed", { provider })}
            </p>
            <p className="m-0 text-muted">{failure.reason}</p>
            <p className="m-0">{t("cloud.sheet.failedLocal")}</p>
            {failure.leftDevice && (
              <p className="m-0 text-muted">{t("cloud.sheet.leftDevice")}</p>
            )}
          </div>
        )}
        <p className="text-small m-0 text-faint">{t("cloud.alwaysAsk")}</p>
      </div>
    </Dialog>
  );
}

/** The first lines of an excerpt: the box shows a sample, "Show exact data" the whole request. */
const clip = (text: string) => text.split("\n").slice(0, 6).join("\n");

const KINDS = [
  "person",
  "org",
  "email",
  "phone",
  "url",
  "id",
  "card",
  "account",
  "term",
] as const;

/** The core's redaction kind (`PERSON`, `EMAIL`, …) in words. */
function kindLabel(t: TFunction, kind: string): string {
  const k = kind.toLowerCase() as (typeof KINDS)[number];
  return KINDS.includes(k) ? t(`cloud.sheet.kinds.${k}`) : kind;
}
