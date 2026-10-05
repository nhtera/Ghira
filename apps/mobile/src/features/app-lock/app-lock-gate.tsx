// SPDX-License-Identifier: Apache-2.0
// The app-lock gate: the top layer above every route and every sheet while the
// app is locked. Before Rust has answered (and when the page is hidden with the
// lock on) it is a blank cover, so no meeting content shows. Unlocking is Face
// ID through `unlock`; the gate asks once when it appears and then on the button.
// The same layer carries the can't-open-your-meetings and couldn't-start screens
// (store-problem), so those never leave a blank page either.
// While it shows, every other child of <body> (the app root, open sheets) is
// inert, and the gate takes pointer events back even if a sheet had turned them off.
import { Icon } from "@ghi/ui";
import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { ipc } from "../../ipc";
import { PROBLEM_TITLE_ID, StoreProblemScreen } from "../store-problem";
import { Btn } from "../settings/controls";
import { knownError } from "../settings/api";
import {
  applyLockEvent,
  markUnlocked,
  pageVisibility,
  refreshLock,
  useLock,
} from "./lock-store";

export { UNLOCKED_EVENT, LOCKED_EVENT } from "./events";

const GATE_ATTR = "data-lock-gate";

/** Makes everything in <body> but the gate inert; puts it back afterwards. */
function useInertBehind(active: boolean) {
  useEffect(() => {
    if (!active) return;
    const changed = new Set<Element>();
    const apply = () => {
      for (const el of Array.from(document.body.children)) {
        if (
          el.hasAttribute(GATE_ATTR) ||
          changed.has(el) ||
          (el as HTMLElement).inert
        )
          continue;
        (el as HTMLElement).inert = true;
        changed.add(el);
      }
    };
    apply();
    const mo = new MutationObserver(apply);
    mo.observe(document.body, { childList: true });
    return () => {
      mo.disconnect();
      changed.forEach((el) => ((el as HTMLElement).inert = false));
    };
  }, [active]);
}

export function AppLockGate() {
  const { t } = useTranslation();
  const { phase, covered, problem } = useLock();
  const [failure, setFailure] = useState<string | null>(null);
  const asking = useRef(false);
  const asked = useRef(false);
  const button = useRef<HTMLButtonElement>(null);
  const locked = phase === "locked";
  const trouble = phase === "unavailable" || phase === "failed";

  const unlock = useCallback(async () => {
    if (asking.current) return;
    asking.current = true;
    setFailure(null);
    try {
      const r = await ipc.commands.unlock(t("mobile.lock.unlockReason"));
      if (r.status === "ok" && r.data) markUnlocked();
      else
        setFailure(
          r.status === "error" && knownError(r.error) === "noAuthMethod"
            ? "noAuthMethod"
            : "failed",
        );
    } catch {
      setFailure("failed");
    } finally {
      asking.current = false;
    }
  }, [t]);

  // Ask again whenever the app comes back to the front; cover when it leaves.
  useEffect(() => {
    void refreshLock();
    const onVisibility = () =>
      pageVisibility(document.visibilityState === "visible");
    const onFocus = () => void refreshLock();
    let off: (() => void) | undefined;
    let alive = true;
    void ipc.onLockChanged(applyLockEvent).then((u) => (alive ? (off = u) : u()));
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("focus", onFocus);
    return () => {
      alive = false;
      off?.();
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("focus", onFocus);
    };
  }, []);

  // One automatic Face ID prompt per lock.
  useEffect(() => {
    if (locked && !asked.current) {
      asked.current = true;
      void unlock();
    }
    if (!locked) asked.current = false;
  }, [locked, unlock]);

  const shown = phase !== "unlocked" || covered;
  useInertBehind(shown);
  useEffect(() => {
    if (locked) button.current?.focus();
  }, [locked]);

  if (!shown) return null;
  return createPortal(
    <div
      {...{ [GATE_ATTR]: "" }}
      role={locked || trouble ? "dialog" : undefined}
      aria-modal={locked || trouble ? true : undefined}
      aria-labelledby={locked ? "lock-title" : trouble ? PROBLEM_TITLE_ID : undefined}
      aria-hidden={locked || trouble ? undefined : true}
      data-testid={trouble ? "store-problem-gate" : "app-lock-gate"}
      style={{ pointerEvents: "auto" }}
      className={
        trouble
          ? "fixed inset-0 z-[1000] flex flex-col items-center overflow-y-auto bg-bg px-8 pt-safe pb-safe text-center text-ink"
          : "fixed inset-0 z-[1000] flex flex-col items-center justify-center gap-4 bg-bg px-8 pt-safe pb-safe text-center text-ink"
      }
    >
      {trouble && <StoreProblemScreen problem={phase === "unavailable" ? problem : null} />}
      {locked && (
        <>
          <span
            aria-hidden="true"
            className="grid size-16 place-items-center rounded-full bg-accent-soft text-accent"
          >
            <Icon name="lock" size={32} />
          </span>
          <h1 id="lock-title" className="text-ios-title2 m-0">
            {t("mobile.lock.title")}
          </h1>
          <p className="text-ios-body m-0 text-muted">
            {t("mobile.lock.body")}
          </p>
          {failure && (
            <p role="alert" className="text-ios-subhead m-0 text-rec-ink">
              {failure === "noAuthMethod"
                ? t("mobile.settings.error.noAuthMethod")
                : t("mobile.lock.failed")}
            </p>
          )}
          <Btn
            ref={button}
            tone="primary"
            onClick={() => void unlock()}
            className="min-w-[14rem]"
          >
            {t("mobile.lock.unlock")}
          </Btn>
        </>
      )}
    </div>,
    document.body,
  );
}
