// SPDX-License-Identifier: Apache-2.0
// Full-screen states with a way forward, for when the app can't show anything
// else: the encrypted store can't be opened ("unavailable", with the reason by
// code), the startup calls keep failing ("failed"), or a screen threw
// (`LoadFailed`, from the error boundary and the router). Never blank: each has
// a message, "Try again", and, for a store that can't be opened, "Start fresh"
// behind the typed phrase the Privacy screen uses for "Delete everything".
import { Icon } from "@ghi/ui";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { StoreProblem } from "../../bindings";
import { ipc } from "../../ipc";
import { retryStartup } from "../app-lock/lock-store";
import { clearRecentSearches } from "../search/recent";
import { errCode, unwrap } from "../settings/api";
import { Btn, ErrorLine, Field } from "../settings/controls";
import { useGo } from "../settings/go";
import { isDeletePhrase } from "../settings/fold";

export const PROBLEM_TITLE_ID = "store-problem-title";

/** The centred column: scrolls instead of clipping at large text sizes. */
function Column({ children }: { children: ReactNode }) {
  return (
    <div className="my-auto flex w-full max-w-md flex-col items-center gap-4 py-6">
      {children}
    </div>
  );
}

function Heading({ children }: { children: ReactNode }) {
  return (
    <h1 id={PROBLEM_TITLE_ID} tabIndex={-1} className="text-ios-title2 m-0 outline-none">
      {children}
    </h1>
  );
}

function Mark() {
  return (
    <span
      aria-hidden="true"
      className="grid size-16 shrink-0 place-items-center rounded-full bg-rec-soft text-rec-ink"
    >
      <Icon name="error" size={32} />
    </span>
  );
}

/** The data is gone for good: only these two may offer "Start fresh" (Rust refuses the rest too). */
const CAN_START_FRESH: readonly StoreProblem[] = ["keyMissing", "damaged"];

/** "Try again": runs `retry`; if the screen is still there afterwards, says so (`already`: it was before, too). */
function useRetry(retry: () => Promise<unknown> | void, already = false) {
  const [busy, setBusy] = useState(false);
  const [still, setStill] = useState(already);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const run = async () => {
    setBusy(true);
    setStill(false);
    try {
      await retry();
    } catch {
      /* the screen stays; say so below */
    }
    if (alive.current) {
      setBusy(false);
      setStill(true);
    }
  };
  return { busy, still, run };
}

/**
 * The gate's content when the store can't be opened (`problem`), or when the
 * startup keeps failing (`problem` null). Rendered inside the lock gate's
 * dialog, which makes everything else inert.
 */
export function StoreProblemScreen({ problem: reported }: { problem: StoreProblem | null }) {
  let problem = reported;
  const { t } = useTranslation();
  const go = useGo();
  const [step, setStep] = useState<"main" | "confirm">("main");
  const [typed, setTyped] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [wiping, setWiping] = useState(false);
  const retry = useRetry(retryStartup);
  const heading = useRef<HTMLDivElement>(null);
  const phrase = t("mobile.privacy.deleteAll.phrase");
  // `startup` (the store opened, starting on it failed) is the generic screen.
  if (problem === "startup") problem = null;
  const canStartFresh = problem !== null && CAN_START_FRESH.includes(problem);

  // Focus lands on the new step's text, so VoiceOver reads it.
  useEffect(() => {
    heading.current?.querySelector<HTMLElement>("h1")?.focus();
  }, [step]);

  const startFresh = async () => {
    setWiping(true);
    setError(null);
    try {
      unwrap(await ipc.commands.storeStartFresh(typed));
      clearRecentSearches();
      go("/onboarding", { replace: true });
      await retryStartup();
    } catch (e) {
      setError(errCode(e));
    } finally {
      setWiping(false);
    }
  };

  if (canStartFresh && step === "confirm") {
    return (
      <Column>
        <div ref={heading} className="flex flex-col items-center gap-4">
          <Heading>{t("mobile.storeProblem.confirm.title")}</Heading>
          <p className="text-ios-body m-0 text-muted">{t("mobile.storeProblem.confirm.body")}</p>
        </div>
        <Field
          id="start-fresh-confirm"
          label={t("mobile.storeProblem.confirm.typePrompt", { phrase })}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="characters"
          spellCheck={false}
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
          className="w-full"
        />
        <ErrorLine code={error} />
        <Btn
          tone="danger"
          className="w-full"
          disabled={!isDeletePhrase(typed) || wiping}
          onClick={() => void startFresh()}
        >
          {t("mobile.storeProblem.confirm.action")}
        </Btn>
        <Btn
          className="w-full"
          disabled={wiping}
          onClick={() => {
            setStep("main");
            setTyped("");
            setError(null);
          }}
        >
          {t("mobile.storeProblem.confirm.cancel")}
        </Btn>
      </Column>
    );
  }

  return (
    <Column>
      <Mark />
      <div ref={heading} className="flex flex-col items-center gap-4">
        <Heading>
          {problem === null ? t("mobile.storeProblem.failed.title") : t("mobile.storeProblem.title")}
        </Heading>
        <p className="text-ios-body m-0 text-muted">
          {problem === null
            ? t("mobile.storeProblem.failed.body")
            : t(`mobile.storeProblem.reason.${problem}`)}
        </p>
      </div>
      {problem !== null && (
        <p className="text-ios-footnote m-0 text-muted" data-testid="store-problem-code">
          {t("mobile.storeProblem.code", { code: problem })}
        </p>
      )}
      {retry.still && (
        <p role="alert" className="text-ios-subhead m-0 text-rec-ink">
          {problem === null
            ? t("mobile.storeProblem.failed.stillBroken")
            : t("mobile.storeProblem.stillBroken")}
        </p>
      )}
      <Btn tone="primary" className="w-full" disabled={retry.busy} onClick={() => void retry.run()}>
        {t("mobile.storeProblem.retry")}
      </Btn>
      {canStartFresh && (
        <>
          <p className="text-ios-footnote m-0 text-muted">{t("mobile.storeProblem.startFreshNote")}</p>
          <Btn tone="danger" className="w-full" onClick={() => setStep("confirm")}>
            {t("mobile.storeProblem.startFresh")}
          </Btn>
        </>
      )}
    </Column>
  );
}

/**
 * A screen threw, or a route failed to load: the same frame, outside the gate.
 * `already`: this is the screen after a "Try again" that failed again.
 */
export function LoadFailed({ onRetry, already }: { onRetry: () => Promise<unknown> | void; already?: boolean }) {
  const { t } = useTranslation();
  const retry = useRetry(onRetry, already);
  const title = useRef<HTMLDivElement>(null);
  useEffect(() => {
    title.current?.querySelector<HTMLElement>("h1")?.focus();
  }, []);
  return (
    <div
      role="alert"
      aria-labelledby={PROBLEM_TITLE_ID}
      data-testid="load-failed"
      className="fixed inset-0 z-[900] flex flex-col items-center overflow-y-auto bg-bg px-8 pt-safe pb-safe text-center text-ink"
    >
      <Column>
        <Mark />
        <div ref={title} className="flex flex-col items-center gap-4">
          <Heading>{t("mobile.storeProblem.failed.title")}</Heading>
          <p className="text-ios-body m-0 text-muted">{t("mobile.storeProblem.failed.body")}</p>
        </div>
        {retry.still && (
          <p className="text-ios-subhead m-0 text-rec-ink">{t("mobile.storeProblem.failed.stillBroken")}</p>
        )}
        <Btn tone="primary" className="w-full" disabled={retry.busy} onClick={() => void retry.run()}>
          {t("mobile.storeProblem.retry")}
        </Btn>
      </Column>
    </div>
  );
}

/** A route that doesn't exist: not a failure, so no "Try again", just the way back. */
export function PageNotFound({ onHome }: { onHome: () => void }) {
  const { t } = useTranslation();
  const title = useRef<HTMLDivElement>(null);
  useEffect(() => {
    title.current?.querySelector<HTMLElement>("h1")?.focus();
  }, []);
  return (
    <div
      role="alert"
      aria-labelledby={PROBLEM_TITLE_ID}
      data-testid="page-not-found"
      className="fixed inset-0 z-[900] flex flex-col items-center overflow-y-auto bg-bg px-8 pt-safe pb-safe text-center text-ink"
    >
      <Column>
        <Mark />
        <div ref={title} className="flex flex-col items-center gap-4">
          <Heading>{t("mobile.storeProblem.notFound.title")}</Heading>
          <p className="text-ios-body m-0 text-muted">{t("mobile.storeProblem.notFound.body")}</p>
        </div>
        <Btn tone="primary" className="w-full" onClick={onHome}>
          {t("mobile.storeProblem.notFound.action")}
        </Btn>
      </Column>
    </div>
  );
}
