// SPDX-License-Identifier: Apache-2.0
// Optional recovery key (doc 05 §2.2): 24 words that wrap the master key, so
// losing the OS keychain doesn't lose the meetings. Reachable but never
// blocking: "Set up later" is always there. The core keeps the phrase only
// after the user types it back; a phrase that was shown and left is forgotten.
import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Button, Icon, useToast, usePlatform } from "@ghi/ui";
import { ipc } from "../../ipc";
import { Notice, StepActions, StepFrame, type StepNav } from "./step-frame";

export function RecoveryStep({ nav }: { nav: StepNav }) {
  const { t } = useTranslation();
  const context = usePlatform();
  const { show } = useToast();
  const typedId = useId();
  const [alreadySet, setAlreadySet] = useState(false);
  const [words, setWords] = useState<string[] | null>(null);
  const [typed, setTyped] = useState("");
  const [mismatch, setMismatch] = useState(false);
  const [busy, setBusy] = useState(false);
  // A phrase shown but not confirmed is forgotten when the step is left.
  const pending = useRef(false);

  useEffect(() => {
    let alive = true;
    void ipc.commands.hasRecoveryKey().then((r) => alive && r.status === "ok" && setAlreadySet(r.data));
    return () => {
      alive = false;
      if (pending.current) void ipc.commands.cancelRecoveryKey();
    };
  }, []);

  const fail = (message: string) => show({ tone: "warning", title: t("system.commandFailed", { message }) });
  const create = async () => {
    setBusy(true);
    try {
      const w = await ipc.commands.createRecoveryKey();
      pending.current = true;
      setWords(w);
    } catch (e) {
      fail(String(e));
    } finally {
      setBusy(false);
    }
  };
  const confirm = async () => {
    setBusy(true);
    try {
      const r = await ipc.commands.confirmRecoveryKey(typed.toLowerCase().split(/\s+/).filter(Boolean));
      if (r.status === "error") return fail(r.error);
      if (!r.data) return setMismatch(true);
      pending.current = false;
      show({ tone: "success", title: t("onboarding.recovery.saved") });
      nav.next();
    } finally {
      setBusy(false);
    }
  };

  return (
    <StepFrame title={t("onboarding.recovery.title")} body={t(`onboarding.recovery.body_${context}`)}>
      {alreadySet && (
        <Notice icon="check_circle">{t("onboarding.recovery.alreadySet")}</Notice>
      )}

      {words && (
        <>
          <ol aria-label={t("onboarding.recovery.wordsLabel")} className="m-0 grid list-none grid-cols-3 gap-x-4 gap-y-1.5 rounded-xl bg-surface2 p-4 sm:grid-cols-4">
            {words.map((w, i) => (
              <li key={i} className="text-mono flex gap-1.5 text-[13px]">
                <span className="w-5 text-right text-faint">{i + 1}</span>
                <span>{w}</span>
              </li>
            ))}
          </ol>
          <p className="text-small m-0 flex items-center gap-1.5 text-muted">
            <Icon name="lock" size={14} />
            {t("onboarding.recovery.shownOnce")}
            <Button size="sm" variant="secondary" className="ml-auto" onClick={() => window.print()}>
              {t("onboarding.recovery.print")}
            </Button>
          </p>
          {/* Print-only copy of the words (the rest of onboarding is print:hidden). No network, no file. */}
          {createPortal(
            <div className="hidden bg-white p-10 text-black print:block">
              <h1 className="text-title m-0 mb-4">{t("onboarding.recovery.title")}</h1>
              <ol className="text-mono m-0 grid list-none grid-cols-4 gap-x-6 gap-y-2 p-0 text-[14px]">
                {words.map((w, i) => (
                  <li key={i}>
                    {i + 1}. {w}
                  </li>
                ))}
              </ol>
            </div>,
            document.body,
          )}
          <div className="flex flex-col gap-1.5">
            <label htmlFor={typedId} className="text-[13px] font-medium">
              {t("onboarding.recovery.typeBack")}
            </label>
            <textarea
              id={typedId}
              value={typed}
              rows={3}
              spellCheck={false}
              autoCapitalize="none"
              autoComplete="off"
              aria-invalid={mismatch || undefined}
              onChange={(e) => {
                setTyped(e.target.value);
                setMismatch(false);
              }}
              className="text-mono w-full resize-none rounded-ctl border border-ctl bg-surface px-3 py-2 text-[13px] text-ink"
            />
            {mismatch && (
              <Notice tone="warn" icon="warning">
                {t("onboarding.recovery.mismatch")}
              </Notice>
            )}
          </div>
        </>
      )}

      <StepActions
        nav={nav}
        start={
          !words &&
          !alreadySet && (
            <Button size="lg" variant="primary" icon="lock" disabled={busy} onClick={() => void create()}>
              {t("onboarding.recovery.create")}
            </Button>
          )
        }
        primary={words ? t("onboarding.recovery.check") : alreadySet ? t("common.continue") : t("onboarding.recovery.later")}
        primaryProps={
          words
            ? { disabled: busy || !typed.trim() }
            : alreadySet
              ? undefined
              : { variant: "ghost", className: "text-muted" }
        }
        onPrimary={words ? () => void confirm() : nav.next}
        skip={words ? t("onboarding.recovery.later") : undefined}
        enter={!!words || alreadySet}
      />
    </StepFrame>
  );
}
