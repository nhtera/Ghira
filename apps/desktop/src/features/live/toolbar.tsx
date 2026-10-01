// SPDX-License-Identifier: Apache-2.0
// The strip under the page title: editable meeting title, mode and language
// badges, the consent helper and the per-meeting "Consent confirmed" toggle.
import { APP_NAME } from "@ghi/i18n";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Icon, cn, useToast } from "@ghi/ui";
import type { ReactNode } from "react";
import type { RecordMode } from "../../bindings";
import { ipc } from "../../ipc";
import { useLive } from "../../state/live";

const TITLE_DEBOUNCE_MS = 600;

/** The title saves a moment after typing stops, on blur, and on leaving. */
function TitleField({ meeting, initial }: { meeting: string; initial: string }) {
  const { t } = useTranslation();
  const { show } = useToast();
  const [title, setTitle] = useState(initial);
  const dirty = useRef<string | null>(null);
  const timer = useRef<number | undefined>(undefined);

  const flush = () => {
    window.clearTimeout(timer.current);
    const value = dirty.current;
    if (value == null) return;
    dirty.current = null;
    void ipc.commands.setMeetingTitle(meeting, value).then((r) => {
      if (r.status === "error") show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
      else useLive.getState().setSessionTitle(value);
    });
  };
  const flushRef = useRef(flush);
  useEffect(() => {
    flushRef.current = flush;
  });
  useEffect(() => () => flushRef.current(), []);

  return (
    <input
      value={title}
      onChange={(e) => {
        setTitle(e.target.value);
        dirty.current = e.target.value;
        window.clearTimeout(timer.current);
        timer.current = window.setTimeout(flush, TITLE_DEBOUNCE_MS);
      }}
      onBlur={flush}
      aria-label={t("live.titleLabel")}
      placeholder={t("live.titlePlaceholder")}
      className="text-body h-8 min-w-40 flex-1 rounded-ctl border border-transparent bg-transparent px-2 font-semibold hover:border-ctl focus:border-ctl focus:bg-surface"
    />
  );
}

function Consent({ meeting, initial, compact }: { meeting: string; initial: boolean; compact: boolean }) {
  const { t } = useTranslation();
  const { show } = useToast();
  const [confirmed, setConfirmed] = useState(initial);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(t("live.consent.message", { app: APP_NAME }));
      show({ tone: "success", title: t("live.consent.copied") });
    } catch {
      show({ tone: "warning", title: t("system.commandFailed", { message: t("live.consent.copy") }) });
    }
  };
  const toggle = async () => {
    const next = !confirmed;
    setConfirmed(next);
    const r = await ipc.commands.setConsentConfirmed(meeting, next);
    if (r.status === "error") {
      setConfirmed(!next);
      show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
    } else useLive.getState().setSessionConsent(next);
  };
  return (
    <div className="flex items-center gap-1.5">
      <Button size="sm" icon="content_copy" onClick={() => void copy()} aria-label={compact ? t("live.consent.copy") : undefined} title={compact ? t("live.consent.copy") : t("live.consent.hint")}>
        {compact ? undefined : t("live.consent.copy")}
      </Button>
      <button
        type="button"
        role="switch"
        aria-checked={confirmed}
        onClick={() => void toggle()}
        title={compact ? t("live.consent.confirmed") : undefined}
        aria-label={compact ? t("live.consent.confirmed") : undefined}
        className={cn("text-small inline-flex h-7 items-center gap-1.5 rounded-seg border px-2 font-medium", confirmed ? "border-accent bg-accent-soft text-accent" : "border-ctl text-muted hover:bg-sunk")}
      >
        <Icon name={confirmed ? "check_box" : "check_box_outline_blank"} size={16} />
        {compact ? undefined : t("live.consent.confirmed")}
      </button>
    </div>
  );
}

const Badge = ({ children }: { children: ReactNode }) => <span className="text-small inline-flex h-6 items-center rounded-full bg-sunk px-2.5 font-medium whitespace-nowrap text-muted">{children}</span>;

export function LiveToolbar({ meeting, mode, compact }: { meeting: string; mode: RecordMode; compact: boolean }) {
  const { t } = useTranslation();
  // The session's own mode/language/title (also after a webview reload).
  const session = useLive((s) => s.session);
  const consent = session?.consentConfirmed ?? false;
  const shownMode: RecordMode = session?.mode === "room" ? "room" : session?.mode === "call" ? "call" : mode;
  const language =
    session?.language === "en" ? t("onboarding.languages.english") : session?.language === "vi" ? t("onboarding.languages.vietnamese") : t("record.languages");
  return (
    <div className="flex flex-wrap items-center gap-2">
      <TitleField key={meeting} meeting={meeting} initial={session?.title ?? ""} />
      <Badge>{t(`record.mode.${shownMode}`)}</Badge>
      <Badge>{language}</Badge>
      <Consent key={`c-${meeting}`} meeting={meeting} initial={consent} compact={compact} />
    </div>
  );
}
