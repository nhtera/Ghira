// SPDX-License-Identifier: Apache-2.0
// Per-meeting cloud opt-in. Shows exactly what leaves the device (transcript
// text only, never audio) with a before/after redaction preview; sending is
// bound to the previewed text by the caller (CloudGrant).
import * as Switch from "@radix-ui/react-switch";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { Icon } from "../../icons/icon";
import { Button } from "../../primitives/button";
import { Dialog } from "../../primitives/dialog";
import { Segmented } from "../../primitives/segmented";
import { usePlatformContext } from "../../platform/platform";
import { cn } from "../../utils/cn";

export type CloudSendState = "default" | "sending" | "sent" | "failed";
export type CloudProvider = { id: string; name: string };

export type CloudSendSheetProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  state: CloudSendState;
  providers: CloudProvider[];
  providerId: string;
  onProviderChange: (id: string) => void;
  words: number;
  tokens: number;
  /** Preformatted, e.g. "about $0.04". Omitted when unknown. */
  cost?: string;
  redact: boolean;
  onRedactChange: (redact: boolean) => void;
  /** Transcript excerpt as it is on this device / as it would be sent. */
  rawText: string;
  redactedText: string;
  onSend: () => void;
  onKeepLocal: () => void;
  onRetry?: () => void;
};

export function CloudSendSheet(p: CloudSendSheetProps) {
  const { t, i18n } = useTranslation();
  const ctx = usePlatformContext();
  const redactId = useId();
  const busy = p.state === "sending";
  const provider = p.providers.find((x) => x.id === p.providerId)?.name ?? "";
  const sent = p.redact ? p.redactedText : p.rawText;
  const fmt = new Intl.NumberFormat(i18n.language);

  const footer =
    p.state === "sent" ? (
      <Button variant="secondary" size="lg" onClick={() => p.onOpenChange(false)}>
        {t("cloud.close")}
      </Button>
    ) : (
      <>
        <Button variant="secondary" size="lg" icon="cloud_off" onClick={p.onKeepLocal} disabled={busy}>
          {t("cloud.keepLocal")}
        </Button>
        {p.state === "failed" ? (
          <Button variant="primary" size="lg" icon="refresh" onClick={p.onRetry ?? p.onSend}>
            {t("common.tryAgain")}
          </Button>
        ) : (
          <Button variant="primary" size="lg" icon={busy ? "progress_activity" : "cloud_upload"} onClick={p.onSend} disabled={busy} aria-busy={busy}>
            {busy ? t("cloud.sendingButton") : t("cloud.send")}
          </Button>
        )}
      </>
    );

  return (
    <Dialog
      open={p.open}
      onOpenChange={p.onOpenChange}
      title={t("cloud.title")}
      description={t("cloud.subtitle")}
      dismissible={!busy}
      width={560}
      footer={footer}
    >
      <div data-state={p.state} className="flex flex-col gap-3.5">
        <div className="flex items-center justify-between gap-3">
          <span className="text-label text-muted">{t("cloud.provider")}</span>
          <Segmented
            label={t("cloud.provider")}
            value={p.providerId}
            onChange={p.onProviderChange}
            options={p.providers.map((x) => ({ value: x.id, label: x.name }))}
          />
        </div>

        <section aria-label={t("cloud.whatLeaves", { context: ctx("cloud.whatLeaves") })} className="flex flex-col gap-1.5 rounded-row bg-surface2 p-3 text-[13px]">
          <h3 className="text-label m-0 text-muted">{t("cloud.whatLeaves", { context: ctx("cloud.whatLeaves") })}</h3>
          <p className="m-0 flex items-center gap-2 text-ink">
            <Icon name="description" size={17} className="text-muted" />
            {t("cloud.textOnly", { words: fmt.format(p.words), tokens: fmt.format(p.tokens) })}
          </p>
          <p className="m-0 flex items-center gap-2 text-ink">
            <Icon name="lock" size={17} className="text-accent" />
            <span className="sr-only">{t("cloud.audio")}: </span>
            {t("cloud.audioNever")}
          </p>
          {p.cost && (
            <p className="m-0 flex items-center gap-2 text-muted">
              <Icon name="info" size={17} />
              {t("cloud.cost")}: {p.cost}
            </p>
          )}
        </section>

        <div className="flex items-center gap-2.5">
          <Switch.Root
            id={redactId}
            checked={p.redact}
            onCheckedChange={p.onRedactChange}
            disabled={busy || p.state === "sent"}
            className="relative h-5 w-9 shrink-0 rounded-full bg-ctl transition-colors duration-(--motion-fast) data-[state=checked]:bg-accent disabled:opacity-50"
          >
            <Switch.Thumb className="block size-4 translate-x-0.5 rounded-full bg-surface transition-transform duration-(--motion-fast) data-[state=checked]:translate-x-[18px]" />
          </Switch.Root>
          <label htmlFor={redactId} className="text-body">
            {t("cloud.redact")}
          </label>
        </div>

        <div className="grid gap-2.5 sm:grid-cols-2">
          <PreviewPane title={t("cloud.before", { context: ctx("cloud.before") })} text={p.rawText} />
          <PreviewPane title={t("cloud.after")} text={sent} tone={p.redact ? "safe" : "warn"} />
        </div>
        {!p.redact && (
          <p className="text-small m-0 flex items-center gap-1.5 text-warn">
            <Icon name="warning" size={16} />
            {t("cloud.redactionOff")}
          </p>
        )}

        {busy && (
          <p role="status" className="text-small m-0 flex items-center gap-2 text-muted">
            <Icon name="progress_activity" size={16} className="animate-spin motion-reduce:animate-none" />
            {t("cloud.sending")}
          </p>
        )}
        {p.state === "sent" && (
          <p role="status" className="text-small m-0 flex items-center gap-2 text-warn">
            <Icon name="cloud" size={16} />
            {t("cloud.done")}
          </p>
        )}
        {p.state === "failed" && (
          <p role="alert" className="text-small m-0 flex items-center gap-2 text-rec-ink">
            <Icon name="cloud_off" size={16} />
            {t("cloud.failed", { provider })}
          </p>
        )}
        <p className="text-small m-0 text-faint">{t("cloud.alwaysAsk")}</p>
      </div>
    </Dialog>
  );
}

function PreviewPane({ title, text, tone }: { title: string; text: string; tone?: "safe" | "warn" }) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <span className="text-label text-muted">{title}</span>
      {/* Text node only: transcript content is never parsed (RT-6). */}
      <pre
        className={cn(
          "text-mono m-0 max-h-32 overflow-auto rounded-ctl border bg-sunk p-2.5 whitespace-pre-wrap break-words text-ink",
          tone === "warn" ? "border-warn" : "border-line",
        )}
      >
        {text}
      </pre>
    </div>
  );
}
