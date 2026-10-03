// SPDX-License-Identifier: Apache-2.0
// Cloud send sheet ("Improve with cloud"). It shows exactly what would leave
// the phone (transcript text only, never audio) for one meeting, with
// redaction on or off, a token count and the exact bytes, and nothing is sent
// until the user taps Send: `cloud_preview` only builds the request. A meeting
// with cloud AI off shows a disabled Send; a failed send leaves the notes on
// the phone. Open it from the meeting view: <CloudSheet meetingId open onOpenChange />.
import { Icon, Sheet } from "@ghi/ui";
import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import type { CloudPreview, CloudSendResult, CloudTask } from "../../bindings";
import { ipc } from "../../ipc";
import { unwrap, useResource } from "../settings/api";
import { Btn, Switch } from "../settings/controls";
import { useGo } from "../settings/go";
import { providerName } from "../settings/providers";

export type CloudSheetProps = {
  meetingId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** What to ask the cloud; default: rewrite the notes in the meeting's own language. */
  task?: CloudTask;
  /** The meeting has cloud AI off (skips the preview). Unknown: the preview call finds out. */
  cloudLocked?: boolean;
  /** After a send that went through. */
  onSent?: (result: CloudSendResult) => void;
};

const DEFAULT_TASK: CloudTask = {
  kind: "notes",
  template: null,
  language: "meeting",
};

export function CloudSheet({ open, ...rest }: CloudSheetProps) {
  // Mounted per open: every opening starts from a fresh preview.
  return open ? <CloudFlow {...rest} /> : null;
}

const loadSetup = async () => {
  const [settings, keys, models] = await Promise.all([
    ipc.commands.getSettings(),
    ipc.commands.cloudKeys(),
    ipc.commands.cloudModels(),
  ]);
  return { settings: unwrap(settings), keys: unwrap(keys), models };
};

/** Rust's refusal text for a meeting with cloud AI off (cloud_preview / cloud_send). */
const isMeetingOff = (error: string) => /off for this meeting/i.test(error);

type Preview =
  | { kind: "ready"; preview: CloudPreview }
  | { kind: "locked" }
  | { kind: "error"; code: string };
type Send =
  | { kind: "idle" }
  | { kind: "sending" }
  | { kind: "sent" }
  | { kind: "failed"; leftDevice: boolean };

function CloudFlow({
  meetingId,
  onOpenChange,
  task = DEFAULT_TASK,
  cloudLocked,
  onSent,
}: Omit<CloudSheetProps, "open">) {
  const { t, i18n } = useTranslation();
  const go = useGo();
  const redactId = useId();
  const setup = useResource(loadSetup);
  const [redactChoice, setRedactChoice] = useState<boolean | null>(null);
  const [send, setSend] = useState<Send>({ kind: "idle" });
  const [result, setResult] = useState<{ key: string; value: Preview } | null>(
    null,
  );

  // Provider and model: the saved choice, else the first provider that has a key.
  const data = setup.data;
  const stored = new Set(
    data?.keys.filter((k) => k.stored).map((k) => k.provider),
  );
  const provider = data
    ? data.settings.cloudProvider ||
      data.models.find((m) => stored.has(m.provider))?.provider ||
      ""
    : "";
  const model = data
    ? data.settings.cloudModel ||
      data.models.find((m) => m.provider === provider)?.model ||
      ""
    : "";
  const hasKey = stored.has(provider);
  const redact = redactChoice ?? data?.settings.cloudRedact ?? true;
  const locked = cloudLocked === true;
  // Cloud notes are not offered (Settings -> Cloud notes): the core refuses with "cloudOff".
  const [refused, setRefused] = useState(false);
  const notOffered = refused || (data ? !data.settings.cloudOffered : false);
  const wantsPreview =
    Boolean(data) && !notOffered && hasKey && Boolean(model) && !locked;
  // A caller may pass a fresh task object every render: compare by value.
  const taskKey = JSON.stringify(task);
  const key = `${provider}|${model}|${redact}|${taskKey}`;

  useEffect(() => {
    if (!wantsPreview) return;
    let alive = true;
    ipc.commands
      .cloudPreview(meetingId, {
        provider,
        model,
        task: JSON.parse(taskKey) as CloudTask,
        redact,
        extraNames: [],
      })
      .then(
        (r) => {
          if (!alive) return;
          let value: Preview;
          if (r.status === "ok" && r.data.kind === "preview")
            value = { kind: "ready", preview: r.data };
          else if (r.status === "error" && r.error === "cloudOff") {
            setRefused(true);
            value = { kind: "error", code: "cloudOff" };
          } else if (r.status === "error")
            value = isMeetingOff(r.error)
              ? { kind: "locked" }
              : { kind: "error", code: r.error };
          else value = { kind: "error", code: "noPreview" };
          setResult({ key, value });
        },
        (e: unknown) =>
          alive &&
          setResult({ key, value: { kind: "error", code: String(e) } }),
      );
    return () => {
      alive = false;
    };
  }, [wantsPreview, meetingId, provider, model, taskKey, redact, key]);

  const current = result?.key === key ? result.value : null;
  const preview = current?.kind === "ready" ? current.preview : null;
  const blocked = locked || current?.kind === "locked";
  const sending = send.kind === "sending";

  const doSend = async () => {
    if (!preview) return;
    setSend({ kind: "sending" });
    try {
      const r = unwrap(await ipc.commands.cloudSend(preview.id));
      if (r.kind === "failed") {
        setSend({ kind: "failed", leftDevice: r.leftDevice });
      } else {
        setSend({ kind: "sent" });
        onSent?.(r);
      }
    } catch (e) {
      // Refused before leaving (changed transcript, cloud turned off): nothing left the phone.
      setSend({ kind: "failed", leftDevice: false });
      if (e instanceof Error && e.message === "cloudOff") setRefused(true);
      if (isMeetingOff(String(e instanceof Error ? e.message : e)))
        setResult({ key, value: { kind: "locked" } });
    }
  };

  /** A redaction kind (Rust's placeholder tag, e.g. PERSON) as a plural noun; unknown tags show as they are. */
  const kindLabel = (kind: string): string => {
    switch (kind.toLowerCase()) {
      case "email":
        return t("mobile.cloudSheet.kind.email");
      case "url":
        return t("mobile.cloudSheet.kind.url");
      case "phone":
        return t("mobile.cloudSheet.kind.phone");
      case "id":
        return t("mobile.cloudSheet.kind.id");
      case "card":
        return t("mobile.cloudSheet.kind.card");
      case "account":
        return t("mobile.cloudSheet.kind.account");
      case "person":
        return t("mobile.cloudSheet.kind.person");
      case "org":
        return t("mobile.cloudSheet.kind.org");
      case "term":
        return t("mobile.cloudSheet.kind.term");
      default:
        return kind;
    }
  };
  const done = send.kind === "sent" || send.kind === "failed";
  const cost =
    preview?.costEstUsd != null
      ? new Intl.NumberFormat(i18n.language, {
          style: "currency",
          currency: "USD",
          maximumFractionDigits: 2,
        }).format(preview.costEstUsd)
      : null;

  return (
    <Sheet
      open
      onOpenChange={onOpenChange}
      title={t("mobile.cloudSheet.title")}
      description={t("mobile.cloudSheet.intro")}
      dismissible={!sending}
      detent="large"
      closeLabel={t("mobile.sheet.close")}
      handleLabel={t("mobile.sheet.handle")}
      footer={
        done ? (
          <Btn tone="primary" onClick={() => onOpenChange(false)}>
            {t("mobile.cloudSheet.close")}
          </Btn>
        ) : (
          <>
            <Btn
              tone="primary"
              onClick={() => void doSend()}
              disabled={!preview || blocked || sending}
              aria-busy={sending}
            >
              {sending
                ? t("mobile.cloudSheet.sending")
                : t("mobile.cloudSheet.send")}
            </Btn>
            <Btn onClick={() => onOpenChange(false)} disabled={sending}>
              {t("mobile.cloudSheet.keepLocal")}
            </Btn>
          </>
        )
      }
    >
      <div
        className="flex flex-col gap-3"
        data-cloud-state={
          blocked
            ? "locked"
            : send.kind === "idle"
              ? preview
                ? "ready"
                : "loading"
              : send.kind
        }
      >
        {setup.error && (
          <p role="alert" className="text-ios-subhead m-0 text-rec-ink">
            {t("mobile.cloudSheet.previewFailed")}
          </p>
        )}
        {notOffered && !blocked && (
          <div className="flex flex-col items-start gap-2">
            <p
              role="status"
              className="text-ios-subhead m-0 flex items-start gap-2 text-warn"
            >
              <Icon name="cloud_off" size={20} className="mt-0.5 shrink-0" />
              {t("mobile.cloudSheet.cloudOff")}
            </p>
            <Btn
              onClick={() => {
                onOpenChange(false);
                go("/settings/cloud");
              }}
            >
              {t("mobile.cloudSheet.openSettings")}
            </Btn>
          </div>
        )}
        {blocked && (
          <p
            role="status"
            className="text-ios-subhead m-0 flex items-start gap-2 text-warn"
          >
            <Icon name="cloud_off" size={20} className="mt-0.5 shrink-0" />
            {t("mobile.cloudSheet.locked")}
          </p>
        )}
        {data && !hasKey && !blocked && !notOffered && (
          <div className="flex flex-col items-start gap-2">
            <p className="text-ios-subhead m-0 text-muted">
              {t("mobile.cloudSheet.noKey")}
            </p>
            <Btn
              onClick={() => {
                onOpenChange(false);
                go("/settings/cloud");
              }}
            >
              {t("mobile.cloudSheet.addKey")}
            </Btn>
          </div>
        )}

        {hasKey && !blocked && !notOffered && (
          <>
            <p className="text-ios-subhead m-0 text-muted">
              {t("mobile.cloudSheet.provider", {
                provider: providerName(provider),
                model,
              })}
              {preview &&
                ` · ${t("mobile.cloudSheet.host", { host: preview.host })}`}
            </p>
            <div className="flex min-h-ios-target items-center justify-between gap-3">
              <span id={redactId} className="text-ios-body">
                {t("mobile.cloudSheet.redact")}
              </span>
              <Switch
                checked={redact}
                labelledBy={redactId}
                disabled={sending || done}
                onChange={(on) => setRedactChoice(on)}
              />
            </div>
            {!redact && (
              <p className="text-ios-footnote m-0 flex items-center gap-1.5 text-warn">
                <Icon name="warning" size={16} className="shrink-0" />
                {t("mobile.cloudSheet.redactionOffWarn")}
              </p>
            )}
            {current?.kind === "error" && (
              <p role="alert" className="text-ios-subhead m-0 text-rec-ink">
                {t("mobile.cloudSheet.previewFailed")}
              </p>
            )}
            {!current && (
              <p role="status" className="text-ios-subhead m-0 text-muted">
                {t("mobile.cloudSheet.preparing")}
              </p>
            )}
            {preview && (
              <>
                <p className="text-ios-footnote m-0 text-muted">
                  {[
                    t("mobile.cloudSheet.tokens", {
                      tokens: new Intl.NumberFormat(i18n.language).format(
                        preview.tokensEst,
                      ),
                    }),
                    cost && t("mobile.cloudSheet.cost", { cost }),
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                  {preview.redactions.length > 0 &&
                    ` · ${t("mobile.cloudSheet.redactions", { items: preview.redactions.map((r) => `${kindLabel(r.kind)} ×${r.count}`).join(", ") })}`}
                </p>
                {preview.warnings.length > 0 && (
                  <div
                    role="status"
                    className="text-ios-footnote flex flex-col gap-1 rounded-(--ios-radius-group) bg-warn-soft p-3 text-ink"
                  >
                    <p className="m-0 flex items-center gap-1.5 font-semibold">
                      <Icon
                        name="warning"
                        size={16}
                        className="shrink-0 text-warn"
                      />
                      {t("mobile.cloudSheet.warningsIntro")}
                    </p>
                    <ul className="m-0 list-disc ps-5">
                      {preview.warnings.map((w) => (
                        <li key={w}>{w}</li>
                      ))}
                    </ul>
                  </div>
                )}
                {preview.retentionNote && (
                  <p className="text-ios-footnote m-0 text-muted">
                    {t("mobile.cloudSheet.retention", {
                      note: preview.retentionNote,
                    })}
                  </p>
                )}
                <section
                  aria-labelledby={`${redactId}-exact`}
                  className="flex flex-col gap-1"
                >
                  <h3
                    id={`${redactId}-exact`}
                    className="text-ios-footnote m-0 px-1 font-medium text-muted"
                  >
                    {t("mobile.cloudSheet.exactHeader")}
                  </h3>
                  {/* The exact request body, as a text node only (RT-6). */}
                  <pre
                    tabIndex={0}
                    className="text-ios-footnote m-0 max-h-60 overflow-auto rounded-(--ios-radius-group) bg-sunk p-3 break-words whitespace-pre-wrap text-ink select-text"
                  >
                    {preview.payload}
                  </pre>
                </section>
              </>
            )}
          </>
        )}

        {send.kind === "sent" && (
          <p
            role="status"
            className="text-ios-subhead m-0 flex items-center gap-2 text-accent"
          >
            <Icon name="cloud_done" size={20} />
            {t("mobile.cloudSheet.sent")}
          </p>
        )}
        {send.kind === "failed" && (
          <div
            role="alert"
            className="text-ios-subhead flex flex-col gap-1 text-rec-ink"
          >
            <p className="m-0 flex items-start gap-2">
              <Icon name="cloud_off" size={20} className="mt-0.5 shrink-0" />
              {t("mobile.cloudSheet.failed")}
            </p>
            {send.leftDevice && (
              <p className="text-ios-footnote m-0">
                {t("mobile.cloudSheet.failedLeft", {
                  provider: providerName(provider),
                })}
              </p>
            )}
          </div>
        )}
      </div>
    </Sheet>
  );
}
