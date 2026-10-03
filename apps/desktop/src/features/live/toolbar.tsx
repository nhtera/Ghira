// SPDX-License-Identifier: Apache-2.0
// Pieces of the live header: the editable meeting title, and the consent
// helper (a dismissible hint with the copy button; "Consent confirmed" is a
// per-meeting flag toggled from the More menu).
import { APP_NAME } from "@ghi/i18n";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Icon, useToast } from "@ghi/ui";
import { ipc } from "../../ipc";
import { useLive } from "../../state/live";

const TITLE_DEBOUNCE_MS = 600;

/** The title saves a moment after typing stops, on blur, and on leaving. */
export function TitleField({ meeting, initial }: { meeting: string; initial: string }) {
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
      className="h-9 min-w-30 flex-1 truncate rounded-seg border border-transparent bg-transparent px-1.5 text-[18px] font-semibold hover:border-line2 focus:border-ctl focus:bg-surface @[1000px]:min-w-44"
    />
  );
}

/** The consent flag and the copyable message for this meeting. */
export function useConsent(meeting: string) {
  const { t } = useTranslation();
  const { show } = useToast();
  const confirmed = useLive((s) => s.session?.consentConfirmed ?? false);
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
    const r = await ipc.commands.setConsentConfirmed(meeting, next);
    if (r.status === "error") show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
    else useLive.getState().setSessionConsent(next);
  };
  return { confirmed, copy, toggle };
}

/** Meetings whose hint was dismissed (it stays gone when the screen is left and re-entered). */
const dismissed = new Set<string>();

/** "Let people know you're recording." with the copy button, above the transcript until it is dismissed or consent is confirmed. */
export function ConsentBanner({ meeting }: { meeting: string }) {
  const { t } = useTranslation();
  const { confirmed, copy } = useConsent(meeting);
  const [, setTick] = useState(0);
  if (confirmed || dismissed.has(meeting)) return null;
  return (
    <div data-testid="consent-hint" className="flex flex-none items-center gap-2.5 rounded-ctl bg-surface2 py-1.5 pr-2 pl-3 text-[12.5px] text-muted">
      <Icon name="campaign" size={16} />
      <span className="min-w-0 flex-1">{t("live.consent.hint")}</span>
      <Button size="sm" icon="content_copy" onClick={() => void copy()} className="flex-none">
        {t("live.consent.copy")}
      </Button>
      <button
        type="button"
        aria-label={t("system.dismiss")}
        onClick={() => {
          dismissed.add(meeting);
          setTick((n) => n + 1);
        }}
        className="grid size-[26px] flex-none place-items-center rounded-seg text-faint hover:bg-sunk"
      >
        <Icon name="close" size={16} />
      </button>
    </div>
  );
}
